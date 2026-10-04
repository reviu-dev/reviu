# Reviu, 15 seconds

A 15-second motion graphics film for Reviu: the agent writes, you review, ship with Git, finish in GitHub.

The film is code. `film.js` draws the picture for any instant, `audio.mjs` synthesizes the soundtrack, and both read their timing from `cues.mjs`, so sound and picture cannot drift apart.

## Render

```sh
node promo/audio.mjs     # soundtrack  -> promo/dist/soundtrack.wav
node promo/render.mjs    # film        -> promo/dist/reviu-15s.mp4
```

The render takes about three minutes and writes 1080p at 60 fps. For 4K, which takes about twelve minutes:

```sh
node promo/render.mjs --scale 2 --name reviu-15s-4k
```

`promo/dist/` is ignored by git. Each render also leaves a lossless master there (`reviu-15s.master.mkv`, about 750 MB at 1080p and 2.8 GB at 4K), which is only needed for `--encode` and is safe to delete.

It needs `ffmpeg`, a Chromium (the Playwright headless shell or Google Chrome is found automatically, or set `CHROME_BIN`), and the website dependencies for the Poppins font (`pnpm install` in `website/`). Nothing else is installed.

## Preview

```sh
node promo/render.mjs --preview
```

Prints a local address that plays the film in a browser in real time. Space pauses, the arrow keys step one frame, and the soundtrack plays along after the first click.

## Options

| Flag | Default | What it does |
| --- | --- | --- |
| `--scale 2` | `1` | Render at 3840x2160. |
| `--height 1080` | source height | Scale the delivery file, for a 1080p file from a `--scale 2` master. |
| `--samples 32` | `32` | Exposures per frame at full speed. `1` turns motion blur off. |
| `--shutter 0.75` | `0.75` | Fraction of each frame the shutter stays open. |
| `--grain 1.4` | `1.4` | Film grain, in 8-bit levels. `0` turns it off. |
| `--crf 14` | `14` | H.264 quality of the delivery file. |
| `--silent` | off | Leave the soundtrack out. |
| `--name reviu-15s-4k` | `reviu-15s` | File name of the master and the delivery file. |
| `--workers 4` | up to 8 | Browsers rendering in parallel. |
| `--from 3.75 --to 7.5` | whole film | Render part of the film. |
| `--encode` | | Re-encode the delivery file from the last master without rendering again. |
| `--still 3.75,11.7` | | Write single frames to `promo/dist/stills/`. Add `--blur` for motion blur. |
| `--sheet 7:7.6:16:4` | | Contact sheet of 16 frames between two times, 4 per row. |
| `--verify` | | Check that seeking is frame-accurate. |

## How it renders

Chromium is asked for the picture at an exact time and screenshotted, so the result does not depend on how fast the machine draws. Each frame is exposed several times across the shutter interval and the exposures are averaged in linear light, which gives real motion blur on the whip pans. Frames go to a lossless master and the delivery file is encoded from that.

## Changing it

- Timing and copy: `cues.mjs`.
- Picture: `film.js` and `film.css`. Every scene is a function of time, so `--sheet` and `--still` are the quickest way to check a change.
- Sound: `audio.mjs`.

The fonts and colours come from the product: Poppins from the website, Inter and Lilex from the desktop app, and the dark theme's own diff, status and syntax colours.
