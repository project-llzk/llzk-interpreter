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

On macOS, the local `scripts/build-macos.sh` helper sets the LLVM / SDK /
Homebrew-related environment expected by the Rust LLZK bindings.

## Scope

This interpreter is intentionally narrow. It aims to cover the LLZK surface
used by `noir_llzk` today, not every LLZK dialect operation or every possible
frontend lowering.
