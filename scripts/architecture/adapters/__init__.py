"""Language adapters.

`python_ast` is always available (standard library `ast`). `rust` is gated on
`rustc` / `cargo` being present on PATH and only performs a lexical `use` scan;
when the toolchain is missing it reports `UNKNOWN` / `NOT_RUN` rather than
claiming the code was checked.
"""
