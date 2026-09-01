#!/usr/bin/env bash
# Play-one-file-of-every-format check from the plan's verification section.
#
# Correctness here is not "did it exit 0". Symphonia will happily *succeed* on a
# WavPack file — WavPack embeds the source RIFF/WAVE header, the probe finds it,
# and the compressed payload is then decoded as raw PCM. Metadata comes out
# perfect and the audio comes out as f32::MAX noise.
#
# So every format is checked against ffmpeg as an oracle: same length, and the
# decoded audio must actually match. Silence and garbage both fail.
set -uo pipefail

STARAMP=${STARAMP:-./target/debug/staramp}
LIB=${STARAMP_TEST_LIBRARY:?set STARAMP_TEST_LIBRARY to a directory of music to check against}
SECS=${SECS:-5}
EXTS=${EXTS:-flac mp3 ogg m4a ape wv mpc dsf aif}

[[ -x $STARAMP ]] || { echo "no staramp binary at $STARAMP (set STARAMP=)" >&2; exit 2; }
[[ -d $LIB ]] || { echo "library not mounted at $LIB (set STARAMP_TEST_LIBRARY=)" >&2; exit 2; }
command -v ffmpeg >/dev/null || { echo "ffmpeg is required as the reference decoder" >&2; exit 2; }

work=$(mktemp -d); trap 'rm -rf "$work"' EXIT

cat > "$work/cmp.py" <<'PY'
import sys, math, array

def load(p):
    a = array.array('f')
    with open(p, 'rb') as f:
        a.frombytes(f.read())
    return a

mine, ref = load(sys.argv[1]), load(sys.argv[2])
n = min(len(mine), len(ref))
if n == 0:
    print("EMPTY no samples"); sys.exit(1)

# Any non-finite sample is decisive: a correct decoder cannot emit one.
bad = sum(1 for x in mine[:n] if not math.isfinite(x))
if bad:
    print(f"GARBAGE {bad} non-finite samples"); sys.exit(1)

rms_mine = math.sqrt(sum(x * x for x in mine[:n]) / n)
rms_ref = math.sqrt(sum(x * x for x in ref[:n]) / n)
diff = math.sqrt(sum((mine[i] - ref[i]) ** 2 for i in range(n)) / n)

if rms_ref > 1e-6 and rms_mine < 1e-6:
    print("SILENCE reference has audio, we produced none"); sys.exit(1)
if diff == 0.0:
    print(f"EXACT bit-identical, rms {rms_mine:.4f}"); sys.exit(0)
if diff < 1e-4 or (rms_ref > 0 and diff < rms_ref * 0.02):
    print(f"MATCH rms {rms_mine:.4f}, diff {diff:.2e} (lossy rounding)"); sys.exit(0)
print(f"MISMATCH rms {rms_mine:.4f} vs {rms_ref:.4f}, diff {diff:.4f}"); sys.exit(1)
PY

pass=0 fail=0 skip=0
printf '%-6s %-9s %-10s %s\n' EXT BACKEND RESULT DETAIL

for ext in $EXTS; do
  file=$(find "$LIB" -maxdepth 6 -iname "*.$ext" -type f 2>/dev/null | head -1)
  if [[ -z $file ]]; then
    printf '%-6s %-9s %-10s %s\n' "$ext" - SKIP "no sample in library"
    ((skip++)); continue
  fi

  probe=$("$STARAMP" probe "$file" 2>&1)
  if [[ $? -ne 0 ]]; then
    printf '%-6s %-9s %-10s %s\n' "$ext" ? FAIL "probe: $(head -1 <<<"$probe" | cut -c1-90)"
    ((fail++)); continue
  fi
  backend=$(awk '/^backend/ {print $2}' <<<"$probe")
  # Compare at the rate staramp actually produces. DSD decodes to hundreds of
  # kHz and is brought down to an allowed rate in the decoder, so the reference
  # has to be asked for the same rate or the two are simply different signals.
  rate=$(awk '/^sample rate/ {print $3}' <<<"$probe")

  if ! "$STARAMP" decode "$file" -o "$work/m.wav" --duration "$SECS" >/dev/null 2>"$work/err"; then
    printf '%-6s %-9s %-10s %s\n' "$ext" "$backend" FAIL "decode: $(head -1 "$work/err" | cut -c1-90)"
    ((fail++)); continue
  fi
  tail -c +45 "$work/m.wav" > "$work/m.raw"
  ffmpeg -v error -i "$file" -t "$SECS" -f f32le -acodec pcm_f32le -ar "$rate" -y "$work/t.raw" 2>/dev/null

  verdict=$(python3 "$work/cmp.py" "$work/m.raw" "$work/t.raw")
  if [[ $? -eq 0 ]]; then
    printf '%-6s %-9s %-10s %s\n' "$ext" "$backend" PASS "$verdict"
    ((pass++))
  else
    printf '%-6s %-9s %-10s %s\n' "$ext" "$backend" FAIL "$verdict"
    ((fail++))
  fi
done

echo
echo "$pass passed, $fail failed, $skip skipped"
[[ $fail -eq 0 ]]
