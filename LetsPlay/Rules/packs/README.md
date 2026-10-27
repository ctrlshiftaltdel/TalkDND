# Content packs

This directory is for local/SRD pack files used by the content loader.

- Pack format, commands and the full-pack builder: `../../docs/CONTENT-PACKS.md`
  (relative to the combat-sim crate).
- Do not commit full-book text or scanned pages here. Full-book packs are
  release artifacts built from the matching local pdf2md index under the
  owner's licence. Keep build output outside Git (for example
  `target/full-packs/`), and load a downloaded pack directory with
  `combat-sim content pack verify --lib <dir>`.
- Players still provide the matching PDFs for the v1.2 unlock flow. The PDF is
  used to identify the book, not to check a hash.
- SRD-derived packs may be kept here if they are CC-BY-4.0 and attributed in
  `NOTICE`.
