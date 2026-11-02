"""Live audio test using only silent virtual sinks, synthetic tones, and temporary files.
Requires Python/numpy and pactl/parec/paplay. Run after cargo build --release.
"""
import json
import os
from pathlib import Path
import signal
import subprocess as sp
import tempfile
import time
import wave
import numpy as np

BIN = str(Path(__file__).resolve().parents[1] / 'target/release/notta-discord')
def pactl(*args):
    return sp.check_output(['pactl', *args], text=True).strip()
def devices(kind):
    return json.loads(pactl('-f', 'json', 'list', kind))
def gone():
    assert not any(x['name'].startswith('notta_discord_') for k in ['sources', 'sinks'] for x in devices(k))
def ready(proc):
    for _ in range(100):
        if proc.poll() is not None:
            raise AssertionError(proc.communicate())
        if any(x['name'] == 'notta_discord_mic' for x in devices('sources')):
            time.sleep(.3)
            return
        time.sleep(.05)
    raise AssertionError('startup timeout')
def launch(mic, output, *args):
    return sp.Popen([BIN, 'run', '--mic', mic + '.monitor', '--output', output, *args], stdout=sp.PIPE, stderr=sp.PIPE, text=True)

def main():
    gone()  # Never interfere with an existing session.
    defaults = [pactl('get-default-source'), pactl('get-default-sink')]
    modules, children = [], []
    tag = str(os.getpid())
    mic, output = 'notta_test_mic_' + tag, 'notta_test_output_' + tag
    try:
        for name in [mic, output]:
            modules.append(pactl('load-module', 'module-null-sink', 'sink_name=' + name, 'rate=48000', 'channels=2'))
        app = launch(mic, output)
        children.append(app)
        ready(app)
        assert [pactl('get-default-source'), pactl('get-default-sink')] == defaults
        for action in ['run', 'cleanup']:
            assert sp.run([BIN, action], capture_output=True).returncode != 0
        with tempfile.TemporaryDirectory() as tmp:
            data = Path(tmp) / 'mix.raw'
            with data.open('wb') as dest:
                record = sp.Popen(['parec', '--device=notta_discord_mic', '--format=float32le', '--rate=48000', '--channels=2'], stdout=dest)
                children.append(record)
                time.sleep(.5)
                for name, hz in [(mic, 440), (output, 880)]:
                    samples = (np.sin(2 * np.pi * hz * np.arange(48000 * 3) / 48000) * 8000).astype('<i2')
                    path = Path(tmp) / (name + '.wav')
                    with wave.open(str(path), 'wb') as wav:
                        wav.setparams((2, 2, 48000, 0, 'NONE', 'not compressed'))
                        wav.writeframes(np.repeat(samples[:, None], 2, axis=1).tobytes())
                    play = sp.Popen(['paplay', '--device=' + name, str(path)])
                    children.append(play)
                for play in children[-2:]:
                    assert play.wait(timeout=10) == 0
                record.terminate()
                record.wait(timeout=5)
            samples = np.fromfile(data, dtype='<f4').reshape(-1, 2)[:, 0]
            # Analyze the central second, where both tones are active.
            center = len(samples) // 2
            segment = samples[center - 24000:center + 24000]
            spectrum = np.abs(np.fft.rfft(segment * np.hanning(len(segment))))
            amplitudes = [float(spectrum[f - 2:f + 3].max()) for f in [440, 880]]
            assert min(amplitudes) > 20, amplitudes
            assert .5 < amplitudes[0] / amplitudes[1] < 2, amplitudes
            print('PASS: mixed 440 Hz + 880 Hz:', amplitudes)
        app.send_signal(signal.SIGINT)
        assert app.wait(timeout=5) == 0, app.communicate()
        gone()
        print('PASS: Ctrl+C cleanup, defaults unchanged, duplicate/cleanup lock')
        app = launch(mic, output)
        children.append(app)
        ready(app)
        app.kill()
        app.wait(timeout=5)
        sp.run([BIN, 'cleanup'], check=True)
        gone()
        print('PASS: recovery after SIGKILL')
        app = launch(mic, output)
        children.append(app)
        ready(app)
        pactl('unload-module', modules.pop())
        assert app.wait(timeout=5) != 0
        gone()
        print('PASS: device disconnection cleanup')
        assert sp.run([BIN, 'run', '--mic', 'missing-device'], capture_output=True).returncode != 0
        gone()
        assert [pactl('get-default-source'), pactl('get-default-sink')] == defaults
    finally:
        for p in reversed(children):
            if p.poll() is None:
                p.terminate()
                p.wait(timeout=5)
        sp.run([BIN, 'cleanup'], capture_output=True)
        for module in reversed(modules):
            pactl('unload-module', module)

if __name__ == '__main__':
    main()
