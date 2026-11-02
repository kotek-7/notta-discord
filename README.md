# notta-discord

Linux のマイク入力と再生音を混ぜて、Notta Web へ渡す Rust CLI。
このPCの PipeWire 1.0.5 / PulseAudio 互換サーバーで動作確認済み。
Rust が `pactl` で音声経路を管理し、ミキシングは音声サーバーが行います。
ローカル音声ファイルは作らず、Notta 側で録音します。Windows/macOS は対象外です。

```text
通常のマイク ───────────────┐
                           ├─ Notta Mix（仮想マイク）→ Notta Web
Discordなど → 通常の出力 ───┘
                  └─ ヘッドホンで聴く
```

## 起動

Rust / Cargo と `pactl` が必要です。Ubuntu 系では `sudo apt install pulseaudio-utils`。
デスクトップにログインしたユーザーで実行してください。sudo は不要です。

```sh
cd ~/dev/notta-discord
cargo build --release
./target/release/notta-discord run
```

ビルド済みなので、以後は最後のコマンドだけで起動できます。
起動したターミナルは会議中そのままにしてください。

1. 使用するヘッドホン・マイクを接続し、普段の入出力として設定します。
2. ツールを起動し、「準備完了」を待ちます。
3. **Discord の入力には通常のマイク**、出力には普段のヘッドホンを指定します。Discord に Notta Mix を選ぶと相手の声を送り返してしまいます。
4. Notta Web の録音設定で、入力マイクを **Notta Mix** にします。表示されなければページを再読み込みし、マイク権限を許可します。Notta 内で選択できない場合はブラウザのマイク設定で指定してください。Chrome の設定画面は `chrome://settings/content/microphone` です。
5. Notta で録音を開始します。短いテスト会話で自分・相手の声が両方入ることを確かめてください。
6. Notta の録音を止めてから、ターミナルで **Ctrl+C**。仮想デバイスが削除されます。

OS の既定入力・出力や、既存アプリの接続先は変更しません。
ブラウザのマイク選択は他のWebサイトにも影響する場合があるので、使用後は元に戻してください。

## デバイス指定・音量

```sh
./target/release/notta-discord devices
./target/release/notta-discord run --mic INPUT_NAME --output OUTPUT_NAME
./target/release/notta-discord run --mic-volume 80 --output-volume 60
```

`devices` に表示されるデバイス名を指定します。マイクは通常 `.monitor` の付かない入力です。
指定を省略すると、起動時の既定マイク・既定出力を使います。
会議アプリの出力先が異なる場合は、その出力を `--output` に指定してください。
音量は各 0〜100%、初期値 70%。物理マイクやスピーカーの音量は変更せず、ミックスに送る音だけを調整します。
音割れする場合は下げてください。リミッター・ノイズ除去・エコーキャンセルは搭載していません。

- 選んだ出力の **すべての再生音** が入ります。通知や音楽も含みます。
- ヘッドホン推奨。スピーカーを使うと、マイクが相手の声を拾い直して二重になる場合があります。
- Discord のミュートは Discord への送信だけを止めます。このツールはマイクを別途取り込むため、Notta 側にも声を入れたくない場合は物理マイクをミュートするか、Notta の録音を一時停止してください。
- 途中でデバイスを切り替える場合はツールを終了して再起動し、Notta の入力を選び直してください。選択中のデバイスが消えた場合は停止・後片付けします。
- 録音は参加者の了承を得て行ってください。

## 強制終了後の復旧

Ctrl+C / SIGTERM 時は自動で削除します。SIGKILL やクラッシュで残った場合：

```sh
./target/release/notta-discord cleanup
```

このツール専用のモジュールだけを削除します。二重起動や、実行中の別プロセスからの cleanup は拒否します。
システムサービス・自動起動設定は追加しません。

## 検証

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build --release
python3 tests/audio_smoke.py
```

音声テストは `numpy`、`paplay`、`parec` が必要です。
一時的な無音出力デバイスを2つ作り、440 Hz / 880 Hz の合成音をそれぞれ流して、
仮想マイクの録音から両方の周波数を検出します。人の声は録音せず、一時ファイルは削除します。
既定デバイスの維持、二重起動防止、Ctrl+C、強制終了後の復旧、デバイス切断時の後片付けも確認します。
Notta アカウントへのログイン、Notta の録音開始・文字起こし結果、実際のDiscord会議は自動テストの対象外です。

## 参照

- [PipeWire: Loopback](https://docs.pipewire.org/page_pulse_module_loopback.html)
- [PipeWire: Remap Source](https://docs.pipewire.org/page_pulse_module_remap_source.html)
- [PipeWire: Null Sink](https://docs.pipewire.org/page_pulse_module_null_sink.html)
- [Notta: マイクが使えない場合の対処法](https://support.notta.ai/hc/ja/articles/38286960178587)
