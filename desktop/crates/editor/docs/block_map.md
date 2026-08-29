# Editor BlockMap prototype

Reviu should keep `Projection` as the diff-line model and move inline UI-only rows behind a smaller block layer instead of adopting Zed's full `DisplayMap`.

## Current prototype

`ProjectionBlockMap` is derived from a projection and indexes non-text blocks:

- folded gaps
- review comment blocks, coalesced across their reserved display lines

Each block carries its display range, kind, and nearest document anchor. Existing rendering still reads `Projection::lines`, but callers can now ask the block map whether a display line belongs to a UI block without matching every display-line variant directly.

## Migration plan

1. Keep deriving `ProjectionBlockMap` from `Projection::lines` while call sites move to block queries.
2. Store the block map on `Projection` once enough call sites depend on it and projection rebuild profiles show repeated derivation cost.
3. Move review comment heights into block metadata so comment or composer height changes can update block ranges without rebuilding diff lines.
4. Move folded gaps to blocks after review comments prove the model, keeping diff hunks and doc-line mapping in `Projection`.

The target is a small Reviu-specific seam: diff lines stay simple, UI blocks become independently measurable and eventually independently invalidated.
