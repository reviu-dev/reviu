# Editor BlockMap prototype

Reviu should keep `Projection` as the diff-line model and move inline UI-only rows behind a smaller block layer instead of adopting Zed's full `DisplayMap`.

## Current prototype

`ProjectionBlockMap` is derived from a projection and indexes non-text blocks:

- folded gaps
- review comment blocks, coalesced across their reserved display lines

Each block carries its display range, reserved height in editor lines, kind, nearest document anchor, and the diff styling metadata needed by editor and gutter rendering for review comments. Start and end folded gaps live only in the block map even though they do not consume display lines. The map also stores a display-line to block index so hot hit-test paths do not scan the block list. `Projection` owns the map derived from its lines, `Editor` keeps a clone next to the active projection, and `PositionMap` carries the same map for pointer handling. Existing text rendering still reads `Projection::lines`, but hit testing, review-comment scroll/layout/create spans, scrollbar review markers, gutter gap/comment styling, editor blank/background/group styling for comment blocks, gap controls, and selection navigation now use typed block-map query helpers instead of matching every display-line variant directly.

## Migration plan

1. Keep deriving interior gap rows from `Projection::lines` while remaining call sites move to block queries.
2. Remove `DisplayLine::Gap` once interior folded gaps can be represented as zero-text block rows without disrupting display-to-doc mapping.

The target is a small Reviu-specific seam: diff lines stay simple, UI blocks become independently measurable and eventually independently invalidated.
