#!/usr/bin/env bash
# End-to-end test against a private PipeWire + WirePlumber (no sound card needed).
#
#   SORDINOD=target/release/sordinod SORDINOCTL=target/release/sordinoctl tests/e2e/run.sh
#
# Needs: pipewire, wireplumber, pw-cat/pw-record/pw-cli/pw-metadata, dbus-run-session, python3 + numpy.
# Everything runs in a throw-away D-Bus session and runtime dir; the host's audio is not touched.
set -u

if [ -z "${E2E_INNER:-}" ]; then
  export E2E_INNER=1
  export XDG_RUNTIME_DIR=$(mktemp -d)
  chmod 700 "$XDG_RUNTIME_DIR"
  # A bus without service directories: an installed Sordino must not be D-Bus-activated into
  # the test session (it would race the daemon under test and outlive the run).
  cat > "$XDG_RUNTIME_DIR/bus.conf" <<'CONF'
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
  exec dbus-run-session --config-file="$XDG_RUNTIME_DIR/bus.conf" -- "$0" "$@"
fi

SORDINOD=$(realpath "${SORDINOD:-target/release/sordinod}")
SORDINOCTL=$(realpath "${SORDINOCTL:-target/release/sordinoctl}")
T=$(mktemp -d)
export XDG_CONFIG_HOME=$T/config XDG_STATE_HOME=$T/state
mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"
PIDS=()
FAILED=0

cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done
  wait 2>/dev/null
}
trap cleanup EXIT

ok()   { echo "  ok   $1"; }
fail() { echo "  FAIL $1"; FAILED=$((FAILED + 1)); }
check() { # check "description" command...
  local d=$1; shift
  if "$@" >/dev/null 2>&1; then ok "$d"; else fail "$d"; fi
}
wait_for() { # wait_for seconds command...
  local n=$(($1 * 5)); shift
  while ! "$@" >/dev/null 2>&1; do n=$((n - 1)); [ $n -le 0 ] && return 1; sleep 0.2; done
}
status() { "$SORDINOCTL" status 2>/dev/null | head -1; }
status_is() { status | grep -q "$1"; }
state() { "$SORDINOCTL" state; }
jget() { state | python3 -c "import json,sys; s=json.load(sys.stdin); print(eval('s'+sys.argv[1]))" "$1"; }

echo "== starting private PipeWire"
pipewire > "$T/pipewire.log" 2>&1 & PIDS+=($!)
wait_for 15 pw-cli info 0 || { echo "pipewire did not start"; cat "$T/pipewire.log"; exit 2; }
wireplumber > "$T/wireplumber.log" 2>&1 & PIDS+=($!)
wait_for 15 pw-metadata -n default || { echo "wireplumber did not start"; cat "$T/wireplumber.log"; exit 2; }

echo "== test signal: 1 kHz sine as a microphone"
python3 - "$T/sine.wav" <<'PY'
import sys, wave, numpy as np
t = np.arange(48000 * 90) / 48000
x = (0.3 * np.sin(2 * np.pi * 1000 * t) * 32767).astype("<i2")
w = wave.open(sys.argv[1], "wb"); w.setnchannels(1); w.setsampwidth(2); w.setframerate(48000); w.writeframes(x.tobytes()); w.close()
PY
start_mic() {
  pw-cat -p --rate 48000 --channels 1 -P 'media.class=Audio/Source node.name=e2e_mic node.description=E2E' "$T/sine.wav" >/dev/null 2>&1 &
  MIC_PID=$!
  PIDS+=($MIC_PID)
}
start_mic
wait_for 10 pw-cli ls Node || true
sleep 1

echo "== daemon start"
"$SORDINOD" > "$T/sordinod.log" 2>&1 & DAEMON=$!
PIDS+=($DAEMON)
wait_for 20 "$SORDINOCTL" status || { echo "daemon did not start"; cat "$T/sordinod.log"; exit 2; }
check "Sordino Mic exists as a node" wait_for 10 sh -c "pw-cli ls Node | grep -q 'node.name = \"sordino_mic\"'"
check "version string carries the attribution" sh -c "'$SORDINOD' --version | grep -q 'Sordino by BxnnyG'"

echo "== settings round trip"
"$SORDINOCTL" set '{"mic":"e2e_mic","noise":{"enabled":false},"studio":{"preset":"off"},"echo":{"enabled":false}}'
check "daemon reaches 'running' with the test microphone" wait_for 15 sh -c "'$SORDINOCTL' status | head -1 | grep -q running"
check "settings are persisted" grep -q 'e2e_mic' "$XDG_CONFIG_HOME/sordino/config.toml"
check "invalid patch is rejected" sh -c "! '$SORDINOCTL' set '{\"noise\":{\"strength\":\"bogus\"}}'"

echo "== audio continuity (sine through Sordino Mic)"
sleep 3
pw-record --target sordino_mic --format f32 --rate 48000 --channels 1 "$T/out.wav" & REC=$!
sleep 10
kill -INT $REC; wait $REC 2>/dev/null
python3 - "$T/out.wav" <<'PY' && ok "output is a continuous 1 kHz sine (no dropouts, no phase jumps)" || fail "output is a continuous 1 kHz sine"
import sys, numpy as np
b = open(sys.argv[1], "rb").read(); i = b.index(b"data") + 8
x = np.frombuffer(b[i:], dtype="<f4")[48000:]
assert len(x) > 48000 * 5, "recording too short"
t = np.arange(len(x)) / 48000
z = np.array([(x[k:k+480] * np.exp(-2j*np.pi*1000*t[k:k+480])).sum() for k in range(0, len(x)-480, 480)])
amp = np.abs(z) * 2 / 480
dph = np.diff(np.unwrap(np.angle(z)))
assert amp.min() > 0.9 * np.median(amp), f"amplitude dip {amp.min():.4f} vs {np.median(amp):.4f}"
assert np.abs(dph - np.median(dph)).max() < 0.05, f"phase jump {np.abs(dph - np.median(dph)).max():.3f}"
PY
"$SORDINOCTL" diag | sed 's/^/        /'
d_under=$(jget "['diag']['out_underruns']"); d_skip=$(jget "['diag']['out_skipped']"); d_drop=$(jget "['diag']['in_dropped']")
[ "$d_under" -le 2 ] && [ "$d_skip" = 0 ] && [ "$d_drop" = 0 ] && ok "glitch counters are clean" || fail "glitch counters (underruns=$d_under skipped=$d_skip dropped=$d_drop)"

echo "== mute and panic"
rms_of() {
  pw-record --target sordino_mic --format f32 --rate 48000 --channels 1 "$T/m.wav" & local r=$!
  sleep 3; kill -INT $r; wait $r 2>/dev/null
  python3 - "$T/m.wav" <<'PY'
import sys, numpy as np
b = open(sys.argv[1], "rb").read(); i = b.index(b"data") + 8
x = np.frombuffer(b[i:], dtype="<f4"); x = x[len(x)//3:]
print(round(float(20 * np.log10(np.sqrt((x**2).mean()) + 1e-9)), 1))
PY
}
"$SORDINOCTL" mute on >/dev/null
lvl=$(rms_of); [ "${lvl%.*}" -lt -80 ] && ok "mute: Sordino Mic is silent ($lvl dBFS)" || fail "mute: Sordino Mic is silent ($lvl dBFS)"
"$SORDINOCTL" mute off >/dev/null
lvl=$(rms_of); [ "${lvl%.*}" -gt -30 ] && ok "unmute: the voice is back ($lvl dBFS)" || fail "unmute: the voice is back ($lvl dBFS)"
"$SORDINOCTL" panic on >/dev/null
sleep 1
check "panic: state says panic and muted" sh -c "'$SORDINOCTL' state | grep -q '\"panic\": true'"
lvl=$(rms_of); [ "${lvl%.*}" -lt -80 ] && ok "panic: Sordino Mic is silent ($lvl dBFS)" || fail "panic: Sordino Mic is silent ($lvl dBFS)"
"$SORDINOCTL" panic off >/dev/null
lvl=$(rms_of); [ "${lvl%.*}" -gt -30 ] && ok "panic off: the voice is back ($lvl dBFS)" || fail "panic off: the voice is back ($lvl dBFS)"
# Settings are applied asynchronously: wait for the daemon to report them.
"$SORDINOCTL" mode recording >/dev/null
check "switching to 'recording' loads its settings" wait_for 5 sh -c "'$SORDINOCTL' state | python3 -c 'import json,sys; s=json.load(sys.stdin)[\"settings\"]; sys.exit(not (s[\"mode\"]==\"recording\" and s[\"noise\"][\"strength\"]==\"medium\" and not s[\"noise\"][\"auto_level\"]))'"
"$SORDINOCTL" mode call >/dev/null
check "switching back to 'call' restores it" wait_for 5 sh -c "'$SORDINOCTL' state | python3 -c 'import json,sys; s=json.load(sys.stdin)[\"settings\"]; sys.exit(not (s[\"mode\"]==\"call\" and s[\"noise\"][\"strength\"]==\"high\"))'"
"$SORDINOCTL" set '{"noise":{"enabled":true,"dereverb":"medium","auto_level":true}}' >/dev/null
sleep 3
lvl=$(rms_of); [ "${lvl%.*}" -gt -60 ] && ok "room echo reduction + automatic level: audio still flows ($lvl dBFS)" || fail "room echo reduction + automatic level: audio still flows ($lvl dBFS)"
"$SORDINOCTL" set '{"noise":{"enabled":false,"dereverb":null}}' >/dev/null
check "mute survives a restart of the settings file" sh -c "'$SORDINOCTL' mute on >/dev/null && grep -q 'muted = true' '$XDG_CONFIG_HOME/sordino/config.toml' && '$SORDINOCTL' mute off >/dev/null"

echo "== hotplug"
kill $MIC_PID; wait $MIC_PID 2>/dev/null
check "status becomes 'waiting for the microphone'" wait_for 10 sh -c "'$SORDINOCTL' status | head -1 | grep -q 'waiting for the microphone'"
check "Sordino Mic stays while the microphone is away" sh -c "pw-cli ls Node | grep -q 'node.name = \"sordino_mic\"'"
start_mic
check "daemon recovers when the microphone returns" wait_for 15 sh -c "'$SORDINOCTL' status | head -1 | grep -q running"

echo "== self-monitoring"
# A null sink stands in for the speakers.
SINK_ID=$(pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=e2e_sink media.class=Audio/Sink object.linger=true audio.position=[FL FR] }' 2>/dev/null | sed -n 's/^id: *\([0-9]*\).*/\1/p' | head -1)
wait_for 10 sh -c "pw-cli ls Node | grep -q 'node.name = \"e2e_sink\"'" || echo "  (could not create a null sink)"
busctl --user call io.github.bxnnyg.Sordino /io/github/bxnnyg/Sordino io.github.bxnnyg.Sordino1 SetMonitor b true >/dev/null
check "monitoring turns on" wait_for 5 sh -c "'$SORDINOCTL' state | grep -q '\"monitoring\": true'"
check "monitoring turns itself off when nobody renews it" wait_for 12 sh -c "'$SORDINOCTL' state | grep -q '\"monitoring\": false'"

echo "== monitoring without any output device must not hurt Sordino Mic"
[ -n "$SINK_ID" ] && pw-cli destroy "$SINK_ID" >/dev/null 2>&1
wait_for 5 sh -c "! pw-cli ls Node | grep -q 'node.name = \"e2e_sink\"'"
busctl --user call io.github.bxnnyg.Sordino /io/github/bxnnyg/Sordino io.github.bxnnyg.Sordino1 SetMonitor b true >/dev/null
sleep 4
check "Sordino Mic is unaffected (still running)" sh -c "'$SORDINOCTL' status | head -1 | grep -q running"
check "Sordino Mic node is still there" sh -c "pw-cli ls Node | grep -q 'node.name = \"sordino_mic\"'"
check "the monitor ends up switched off (failed or expired)" wait_for 12 sh -c "'$SORDINOCTL' state | grep -q '\"monitoring\": false'"
check "Sordino Mic is still running afterwards" sh -c "'$SORDINOCTL' status | head -1 | grep -q running"

echo "== default microphone: set and restore"
before=$(pw-metadata -n default 0 default.configured.audio.source 2>/dev/null | grep -c "sordino_mic")
"$SORDINOCTL" default on
check "Sordino Mic becomes the configured default" wait_for 10 sh -c "pw-metadata -n default 0 default.configured.audio.source | grep -q sordino_mic"
"$SORDINOCTL" default off
check "the previous default is restored" wait_for 10 sh -c "! pw-metadata -n default 0 default.configured.audio.source | grep -q sordino_mic"

echo "== Sordino Speaker (cleaning incoming voices)"
OUT_ID=$(pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=e2e_out media.class=Audio/Sink object.linger=true audio.position=[FL FR] }' 2>/dev/null | sed -n 's/^id: *\([0-9]*\).*/\1/p' | head -1)
wait_for 10 sh -c "pw-cli ls Node | grep -q 'node.name = \"e2e_out\"'"
"$SORDINOCTL" default-output e2e_out
"$SORDINOCTL" speaker light
check "Sordino Speaker appears as an output" wait_for 15 sh -c "pw-cli ls Node | grep -q 'node.name = \"sordino_speaker\"'"
check "it plays to the real output" wait_for 10 sh -c "'$SORDINOCTL' state | grep -q '\"speaker_output\": \"e2e_out\"'"
pw-cat -p --rate 48000 --channels 1 --target sordino_speaker "$T/sine.wav" >/dev/null 2>&1 & PLAY=$!
PIDS+=($PLAY)
sleep 3
pw-record --target e2e_out -P stream.capture.sink=true --format f32 --rate 48000 --channels 1 "$T/spk.wav" & REC=$!
sleep 5
kill -INT $REC; wait $REC 2>/dev/null
python3 - "$T/spk.wav" <<'PY' && ok "audio flows through Sordino Speaker to the output" || fail "audio flows through Sordino Speaker to the output"
import sys, numpy as np
b = open(sys.argv[1], "rb").read(); i = b.index(b"data") + 8
x = np.frombuffer(b[i:], dtype="<f4")
x = x[len(x)//4:]
rms = 20 * np.log10(np.sqrt((x**2).mean()) + 1e-9)
assert rms > -45, f"output too quiet: {rms:.1f} dBFS"
PY
kill $PLAY 2>/dev/null
"$SORDINOCTL" default-output sordino_speaker
sleep 2
check "no feedback loop: with Sordino Speaker as default it still plays to the real device" sh -c "'$SORDINOCTL' state | grep -q '\"speaker_output\": \"e2e_out\"'"
check "the microphone side is unaffected" sh -c "'$SORDINOCTL' status | head -1 | grep -q running"
"$SORDINOCTL" speaker off
check "switching it off removes Sordino Speaker" wait_for 10 sh -c "! pw-cli ls Node | grep -q 'node.name = \"sordino_speaker\"'"
[ -n "$OUT_ID" ] && pw-cli destroy "$OUT_ID" >/dev/null 2>&1

echo "== crash safety: kill -9"
kill -9 $DAEMON; wait $DAEMON 2>/dev/null
check "PipeWire still answers" pw-cli info 0
check "WirePlumber is still running" pw-metadata -n default
check "Sordino Mic is gone, nothing else broke" sh -c "! pw-cli ls Node | grep -q 'node.name = \"sordino_mic\"'"
"$SORDINOD" > "$T/sordinod2.log" 2>&1 & DAEMON=$!
PIDS+=($DAEMON)
check "daemon starts again after a crash" wait_for 20 sh -c "'$SORDINOCTL' status | head -1 | grep -q running"

echo
if [ $FAILED -eq 0 ]; then echo "ALL END-TO-END CHECKS PASSED"; else echo "$FAILED END-TO-END CHECK(S) FAILED"; echo "--- sordinod.log"; tail -30 "$T/sordinod.log"; fi
exit $FAILED
