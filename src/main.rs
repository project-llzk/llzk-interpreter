use std::{env, fs};

use llzk::prelude::{LlzkContext, Module};
use llzk_interpreter::{Felt, Interpreter, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Check,
    Compute,
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  llzk-interpreter <file.llzk> <struct_name> [felt_arg_decimals...]");
    eprintln!("  llzk-interpreter check <file.llzk> <struct_name> [felt_arg_decimals...]");
    eprintln!("  llzk-interpreter compute <file.llzk> <struct_name> [felt_arg_decimals...]");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    let mode = match args.first().map(String::as_str) {
        Some("check") => {
            args.remove(0);
            Mode::Check
        }
        Some("compute") => {
            args.remove(0);
            Mode::Compute
        }
        _ => Mode::Check,
    };
    let mut args = args.into_iter();

    let Some(path) = args.next() else {
        print_usage();
        std::process::exit(1);
    };
    let Some(struct_name) = args.next() else {
        print_usage();
        std::process::exit(1);
    };

    let input = fs::read_to_string(path)?;
    let context = LlzkContext::new();
    let module =
        Module::parse(&context, &input).ok_or_else(|| "failed to parse LLZK module".to_string())?;

    let felt_args = args
        .map(|arg| Ok(Value::Felt(Felt::from_decimal(&arg)?)))
        .collect::<Result<Vec<_>, String>>()?;

    let mut interpreter = Interpreter::new(&module);
    let computed = interpreter.run_compute(&struct_name, &felt_args)?;
    println!("{computed}");
    if mode == Mode::Check {
        interpreter.run_constrain(&struct_name, computed, &felt_args)?;
        println!("constraints satisfied");
    }
    Ok(())
}
