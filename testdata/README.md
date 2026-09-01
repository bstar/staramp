# testdata

`tone.wv` is two seconds of a 440 Hz sine, generated with ffmpeg and encoded as
WavPack. It carries no copyright worth worrying about.

WavPack on purpose: it is decoded through libavcodec rather than symphonia, so
running it proves the ffmpeg libraries are present and working. That is exactly
what the release workflow checks the AppImage for on every distribution it can
find, because `--version` only proves the loader found the libraries, not that
they decode anything.

```sh
staramp probe  testdata/tone.wv
staramp decode testdata/tone.wv -o /tmp/out.wav
```
