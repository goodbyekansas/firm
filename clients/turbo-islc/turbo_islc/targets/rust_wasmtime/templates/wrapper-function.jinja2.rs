{% import "wrapper-function-macros.jinja2.rs" as macros with context %}

pub fn {{ function.name | snakecase }}<T: super::{{ trait_name }}>(
    mut caller: wasmtime::Caller<'_, T>,
    {% for arg in function.arguments.values() %}
    {{ types.wrapper_in(arg) | indent(4) }}
    {% endfor %}
    {% for arg in function.return_values.values() %}
    {{ types.wrapper_out(arg) | indent(4) }}
    {% endfor %}
) -> {{ types.abi_size_type }} {
    let mem = *try_or_errmsg!(caller, &get_memory(&mut caller, "memory"));
    let mem_base = mem.data_ptr(&caller);
    let data = caller.data_mut();

    {% for arg in function.arguments.values() %}
    // Convert input "{{arg.name}}"
    let {{ arg.name | snakecase }} = {{ macros.convert_input(arg) | indent(4) }};
    {% endfor %}

    let native_res = try_or_errmsg!(caller, data.{{ function.name | snakecase }}({{ function.arguments.keys() | map('snakecase') | join(", ") }}));

    // Write output values to WASM
    {{ function.return_value_writer.generate(data_source_name="native_res") | indent(4) }}
    0
}

