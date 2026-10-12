# llzk-interpreter

Small interpreter for a subset of LLZK.

Yes, this interpreter was vibecoded.

## Current coverage

Implemented runtime support includes:

- `felt`: `const`, `add`, `sub`, `mul`, `div`, `uintdiv`, `umod`, `neg`,
  `bit_and`, `bit_or`, `bit_xor`, `shl`, `shr`, `pow`
- `bool`: `cmp`, `and`, `or`, `not`, `assert`
- `constrain.eq`
- `function.call`, `function.return`
- `struct.new`, `struct.readm`, `struct.writem`
- `arith`:
  - `constant` (for `index`, `i1`, and arbitrary-width `iN`)
  - arithmetic: `addi`, `subi`, `muli`, `divsi`, `divui`, `remsi`, `remui`,
    `ceildivsi`, `ceildivui`, `floordivsi`
  - bitwise/shift: `andi`, `ori`, `xori`, `shli`, `shrsi`, `shrui`
  - min/max: `maxsi`, `maxui`, `minsi`, `minui`
  - comparison: `cmpi` (all 10 predicates: `eq`, `ne`, `slt`, `sle`, `sgt`,
    `sge`, `ult`, `ule`, `ugt`, `uge`)
  - casts: `extsi`, `extui`, `trunci`, `index_cast`, `index_castui`
  - `select`
- `cast.toindex`, `cast.tofelt`
- `array.new`, `array.read`, `array.write`
- `ram.load`, `ram.store` (flat felt memory; unwritten cells read as zero)
- `scf.if`, `scf.while`, `scf.condition`, `scf.yield`
- `llzk.nondet` (resolved from a pre-supplied FIFO queue via
  `Interpreter::set_nondet_values`, falls back to zero when empty)

## CLI

By default, the binary runs `@compute` and then `@constrain`:

```sh
llzk-interpreter <file.llzk> <struct_name> [felt_arg_decimals...]
```

Explicit modes are also available:

```sh
llzk-interpreter check <file.llzk> <struct_name> [felt_arg_decimals...]
llzk-interpreter compute <file.llzk> <struct_name> [felt_arg_decimals...]
```

`check` is the default mode. It prints the computed struct and then verifies
that `@constrain` succeeds for the same inputs.

Example:

```sh
cargo run -- path/to/program.llzk Circuit0 1 2 3
```

Compute-only example:

```sh
cargo run -- compute path/to/program.llzk Circuit0 1 2 3
```

## Development

Common targets:

```sh
make build
make test
make lint
make fmt
```

### Nix

The flake provides a compatible Rust, LLZK, and LLVM/MLIR toolchain:

```sh
nix build                            # binary: result/bin/llzk-interpreter
nix develop --command cargo build    # build in the development environment
nix develop --command cargo test     # run the test suite
```

Run `nix develop` for an interactive shell, then use Cargo or the Make
targets above. `nix build` also runs the test suite.
Keep the `llzk-rs` revisions in `Cargo.lock` and `flake.lock` aligned when
updating dependencies (`nix flake update llzk-rs` updates the Nix input).

### Manual build

Follow the [llzk-rs manual installation instructions](https://github.com/project-llzk/llzk-rs#manual-installation)
to install LLVM/MLIR, build and install LLZK, and configure the build environment.
Use versions compatible with the `llzk-rs` revision in this repository's
`Cargo.lock`, and use the same LLVM/MLIR build for LLZK and the Rust bindings.
The dependencies are already declared in `Cargo.toml`; Cargo builds the bindings
as part of building the interpreter.

Keep the upstream environment variables set (`MLIR_SYS_230_PREFIX`,
`TABLEGEN_230_PREFIX`, `LLZK_SYS_30_PREFIX`, and any platform-specific settings),
then run these commands from this repository's root:

```sh
make build
make test
```

On macOS, these targets use `scripts/build-macos.sh` to check LLVM/LLZK
compatibility and configure library search paths. You can also invoke it
directly with Cargo arguments:

```sh
./scripts/build-macos.sh                  # cargo build
./scripts/build-macos.sh build --release  # release build
./scripts/build-macos.sh test             # run tests
```

### Troubleshooting

- **Undefined LLVM/MLIR symbols:** check for mixed toolchains. LLZK's
  `CMakeCache.txt` records `LLVM_DIR` and `MLIR_DIR`; its installed
  `lib/cmake/LLZK/LLZKTargets.cmake` also records the header paths.
- **Switched native installations:** run
  `cargo clean -p llzk-sys -p mlir-sys -p tblgen` before rebuilding.

## Scope

This interpreter is intentionally narrow. It aims to cover the LLZK surface
used by `noir_llzk` today, not every LLZK dialect operation or every possible
frontend lowering.
