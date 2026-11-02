use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use fs2::FileExt;
use serde_json::Value;
use std::{
    fs::OpenOptions,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const SINK: &str = "notta_discord_mix";
const SOURCE: &str = "notta_discord_mic";

#[derive(Parser)]
#[command(
    version,
    about = "マイク＋再生音を Notta Mix 仮想マイクへ送る (Linux / PipeWire・PulseAudio)"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// 入出力デバイスを表示
    Devices,
    /// 仮想マイクを起動。Ctrl+C で終了・削除
    Run {
        /// 入力ソース名。省略時は起動時の既定マイク
        #[arg(long)]
        mic: Option<String>,
        /// 再生シンク名。省略時は起動時の既定出力
        #[arg(long)]
        output: Option<String>,
        /// ミックス内のマイク音量 (0–100%)
        #[arg(long, default_value_t = 70, value_parser = clap::value_parser!(u8).range(0..=100))]
        mic_volume: u8,
        /// ミックス内の再生音量 (0–100%)
        #[arg(long, default_value_t = 70, value_parser = clap::value_parser!(u8).range(0..=100))]
        output_volume: u8,
    },
    /// 強制終了後に残った、このツールの仮想デバイスを削除
    Cleanup,
}

fn pactl(args: &[&str]) -> Result<String> {
    let out = Command::new("pactl")
        .args(args)
        .output()
        .context("pactl を実行できません。pulseaudio-utils をインストールしてください")?;
    ensure!(
        out.status.success(),
        "pactl {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}

fn list(kind: &str) -> Result<Vec<Value>> {
    // Older pactl omits module indices from JSON; short format includes them.
    if kind == "modules" {
        return Ok(pactl(&["list", "short", "modules"])?
            .lines()
            .filter_map(|line| {
                let fields: Vec<_> = line.split('\t').collect();
                let id = fields.first()?.parse::<u64>().ok()?;
                Some(serde_json::json!({"index": id, "name": fields.get(1)?, "argument": fields.get(2)?}))
            })
            .collect());
    }
    Ok(serde_json::from_str(&pactl(&[
        "--format=json",
        "list",
        kind,
    ])?)?)
}

fn numeric_id(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn named(kind: &str, name: &str) -> Result<Value> {
    list(kind)?
        .into_iter()
        .find(|v| v["name"] == name)
        .with_context(|| format!("デバイスが見つかりません: {name}。devices で確認してください"))
}

// Values are passed without a shell, but also validate for PulseAudio's module argument parser.
fn device_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c)),
        "使用できないデバイス名: {name}"
    );
    ensure!(
        name != SOURCE && !name.starts_with(SINK),
        "Notta Mix 自体を入力にすると音声がループします"
    );
    Ok(())
}

fn owned(module: &Value) -> bool {
    let name = module["name"].as_str().unwrap_or("");
    let args = module["argument"].as_str().unwrap_or("");
    let token = match name {
        "module-null-sink" => format!("sink_name={SINK}"),
        "module-loopback" => format!("sink={SINK}"),
        "module-remap-source" => format!("source_name={SOURCE}"),
        _ => return false,
    };
    args.split_whitespace().any(|a| a == token)
}

fn cleanup() -> Result<()> {
    let mut modules = list("modules")?;
    modules.reverse();
    let mut failures = Vec::new();
    for m in modules.iter().filter(|m| owned(m)) {
        if let Err(e) = pactl(&["unload-module", &m["index"].to_string()]) {
            failures.push(e.to_string());
        }
    }
    ensure!(failures.is_empty(), "削除失敗: {}", failures.join("; "));
    Ok(())
}

struct Session {
    modules: Vec<(u64, String)>,
}
impl Session {
    fn load(&mut self, name: &str, args: &[String]) -> Result<u64> {
        let mut command = vec!["load-module", name];
        command.extend(args.iter().map(String::as_str));
        let id: u64 = pactl(&command)?.parse()?;
        self.modules.push((id, name.to_owned()));
        Ok(id)
    }

    fn loopback(&mut self, source: &str, volume: u8) -> Result<()> {
        let id = self.load(
            "module-loopback",
            &[
                format!("source={source}"),
                format!("sink={SINK}"),
                "source_dont_move=true".into(),
                "sink_dont_move=true".into(),
                "latency_msec=40".into(),
            ],
        )?;
        for _ in 0..40 {
            let inputs = list("sink-inputs")?;
            if let Some(input) = inputs
                .iter()
                .find(|v| numeric_id(&v["owner_module"]) == Some(id))
            {
                pactl(&[
                    "set-sink-input-volume",
                    &input["index"].to_string(),
                    &format!("{volume}%"),
                ])?;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        bail!("音声ストリームが作成されませんでした (module {id})")
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Ok(modules) = list("modules") {
            for (id, name) in self.modules.iter().rev() {
                // A server restart may reuse IDs. Never unload an unrelated module.
                if modules
                    .iter()
                    .any(|m| m["index"].as_u64() == Some(*id) && m["name"] == *name && owned(m))
                    && let Err(e) = pactl(&["unload-module", &id.to_string()])
                {
                    eprintln!("後片付けに失敗: {e}。cleanup を実行してください");
                }
            }
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(cli.command, Action::Devices) {
        for (kind, default) in [
            ("sources", "get-default-source"),
            ("sinks", "get-default-sink"),
        ] {
            let current = pactl(&[default])?;
            println!(
                "\n{} (* は既定)",
                if kind == "sources" {
                    "入力 / --mic"
                } else {
                    "出力 / --output"
                }
            );
            for v in list(kind)? {
                let name = v["name"].as_str().unwrap_or("");
                println!(
                    "{} {}\n    {}",
                    if name == current { "*" } else { " " },
                    name,
                    v["description"].as_str().unwrap_or("")
                );
            }
        }
        return Ok(());
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR がありません。デスクトップのユーザーで実行してください")?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(std::path::Path::new(&runtime).join("notta-discord.lock"))?;
    lock.try_lock_exclusive()
        .context("すでに実行中です。先に実行中のツールを Ctrl+C で終了してください")?;

    match cli.command {
        Action::Cleanup => {
            cleanup()?;
            println!("Notta Mix を削除しました");
        }
        Action::Run {
            mic,
            output,
            mic_volume,
            output_volume,
        } => {
            let running = Arc::new(AtomicBool::new(true));
            let flag = Arc::clone(&running);
            ctrlc::set_handler(move || flag.store(false, Ordering::SeqCst))?;
            let mic = mic.unwrap_or(pactl(&["get-default-source"])?);
            let output = output.unwrap_or(pactl(&["get-default-sink"])?);
            device_name(&mic)?;
            device_name(&output)?;
            named("sources", &mic)?;
            let sink = named("sinks", &output)?;
            let monitor = sink["monitor_source"]
                .as_str()
                .context("出力の monitor_source がありません")?;
            device_name(monitor)?;
            ensure!(
                mic != monitor,
                "マイクと再生音が同一です。--mic にマイク入力を指定してください"
            );
            ensure!(
                !list("modules")?.iter().any(owned),
                "前回の仮想デバイスが残っています。cleanup を実行してください"
            );
            let mut session = Session {
                modules: Vec::new(),
            };
            session.load(
                "module-null-sink",
                &[
                    format!("sink_name={SINK}"),
                    "rate=48000".into(),
                    "channels=2".into(),
                    "sink_properties=device.description=Notta-Mix-Bus".into(),
                ],
            )?;
            session.loopback(&mic, mic_volume)?;
            session.loopback(monitor, output_volume)?;
            session.load(
                "module-remap-source",
                &[
                    format!("master={SINK}.monitor"),
                    format!("source_name={SOURCE}"),
                    "source_properties=device.description='Notta Mix'".into(),
                    "rate=48000".into(),
                    "channels=2".into(),
                ],
            )?;
            println!(
                "準備完了: Notta の入力に「Notta Mix」を選択してください。\nマイク: {mic} ({mic_volume}%)\n再生音: {output} ({output_volume}%)\nDiscord の入力は通常のマイクを指定してください。終了: Ctrl+C"
            );
            while running.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(500));
                named("sources", &mic)
                    .context("マイクが切断されました。接続後に再起動してください")?;
                named("sinks", &output)
                    .context("出力が切断されました。接続後に再起動してください")?;
                let modules = list("modules")?;
                ensure!(
                    session
                        .modules
                        .iter()
                        .all(
                            |(id, name)| modules.iter().any(|m| m["index"].as_u64() == Some(*id)
                                && m["name"] == *name
                                && owned(m))
                        ),
                    "仮想デバイスが失われました。再起動してください"
                );
            }
            drop(session);
            println!("停止しました。仮想デバイスを削除しました");
        }
        Action::Devices => unreachable!(),
    }
    Ok(())
}
