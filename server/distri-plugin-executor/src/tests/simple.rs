use rustyscript::{json_args, module, Error, Module, Runtime, RuntimeOptions, Undefined};
const SIMPLE_MODULE: Module = module!(
    "simple.ts",
    "
    export type SimpleOutput = {
      input: string;
    };
    export default function simple(input: string): SimpleOutput {
      return { input };
    }
  "
);

const SECOND_MODULE: Module = module!(
    "second.ts",
    "
    import simple from './simple.ts';
    export type SimpleOutput = {
      input: string;
    };
    export default function second(input: string): SimpleOutput {
      return { input: simple(input).input };
    }
  "
);

#[test]
fn test_ts() -> Result<(), Error> {
    // First we need a runtime. There are a handful of options available
    // here but the one we need right now is default_entrypoint.
    // This tells the runtime that a function is needed for initial
    // setup of our runtime.
    let mut runtime = Runtime::new(RuntimeOptions {
        ..Default::default()
    })?;

    let module_handle = runtime.load_module(&SIMPLE_MODULE)?;
    let simple_result =
        runtime.call_entrypoint::<Undefined>(&module_handle, json_args!("hello"))?;

    println!("simple_result: {:?}", simple_result);

    let module_handle = runtime.load_module(&SECOND_MODULE)?;
    let second_result =
        runtime.call_entrypoint::<Undefined>(&module_handle, json_args!("hello"))?;

    println!("second_result: {:?}", second_result);

    Ok(())
}
