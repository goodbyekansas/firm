{% set module_name = module_name | snakecase %}
{% set trait_name = "%sApi" % module_name | camelcase %}

pub mod {{ module_name }} {
        fn create_wasm_string<T>(
            caller: &mut wasmtime::Caller<'_, T>,
            ptr: *const u8,
            len: usize,
        ) -> Result<usize, String> {
            Ok(offset)
        }

    macro_rules! try_or_errmsg {
        ($caller:ident, $expression:expr) => {
            match $expression {
                Ok(value) => value,
                Err(e) => {
                    let error_msg = format!("Error: {}", e);
                    let offset = allocate($caller, error_msg.len() + 1).unwrap_or(-1{{ types.abi_size_type }});
                    if offset != -1 {
                        let mem = &get_memory($caller, "memory")?;
                        unsafe {
                            mem.data_ptr($caller.as_context_mut())
                                .add(offset)
                                .copy_from_nonoverlapping(error_msg.as_ptr(), error_msg.len());
                            *mem.data_ptr($caller.as_context_mut()).add(offset + len) = b'\0';
                        }
                    }
                    return offset;
                }
            }
        };
    }

    #[allow(clippy::needless_lifetimes, clippy::extra_unused_lifetimes)]
    pub trait {{ trait_name }} {
        {% for function in functions.values() %}
            {% filter indent(width=8, first=True) %}
            {% include "trait-function.jinja2.rs" %}
            {{- "\n" if not loop.last else "" -}}
            {% endfilter %}
        {% endfor %}
    }

    {% for name, enum in enums.items() %}
    #[repr(u8)]
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum {{ name | camelcase }} {
        {% for variant in enum %}
        {{ variant | camelcase }} = {{ loop.index0 }},
        {% endfor %}
    }

    impl TryFrom<u8> for {{ name | camelcase }} {
        type Error = String;
        fn try_from(value: u8) -> Result<Self, Self::Error> {
            match value {
                {% for variant in enum %}
                {{ loop.index0 }} => Ok({{ name | camelcase }}::{{ variant | camelcase }}),
                {% endfor %}
                v => Err(format!(r#"Enum "{{ name | camelcase }}" of {} out of bounds."#, v)),
            }
        }
    }
    {% endfor %}

    {% for record in records.values() %}
    #[derive(Clone, Debug)]
    pub struct {{record.name | camelcase }}{{lifetime.struct(record.fields, lifetime_name="a")}} {
        {% for field in record.fields.values() %}
        {{ types.struct_field(field, pub=True, lifetime_name="a") }}
        {% endfor %}
    }

    {% endfor %}
    {% for (function_name, function) in functions.items() if function.is_multi_return() %}
    #[derive(Clone, Debug)]
    pub struct {{ function_name | camelcase }}Result{{lifetime.struct(function.return_values, lifetime_name="a")}} {
        {% for field in function.return_values.values() %}
        {{ types.struct_field(field, pub=True, lifetime_name="a") }}
        {% endfor %}
    }

    {% endfor %}

    pub fn add_to_linker<T: {{ trait_name }} + Clone + Send + Sync + 'static>(linker: &mut wasmtime::Linker<T>) -> Result<(), String> {
        {% for (name, function) in functions.items() %}
        linker
            .func_wrap("{{ module_name }}", "__{{ name | snakecase }}", wrappers::{{ name | snakecase }})
            .map_err(|e| e.to_string())?;
        {% endfor %}
        Ok(())
    }

    #[cfg(test)]
    pub static mut GET_FUNCTION: Option<Box<dyn Fn(&str) -> Option<wasmtime::Func>>> = None;

    #[cfg(test)]
    pub static mut GET_MEMORY: Option<Box<dyn Fn(&str) -> Option<wasmtime::Memory>>> = None;

    #[allow(unused_assignments, unused_variables, dead_code)]
    mod wrappers {
        use wasmtime::AsContextMut;

        {% for record in records.values() %}
        const {{record.name | snakecase | upper }}_SIZE: usize = {% for field in record.fields.values() %}
        {% if field.is_list() -%}
        (std::mem::size_of::<{{ types.abi_size_type }}>() * 2)
        {%- elif field.is_record() %}
        {{- field.type_name() | snakecase | upper }}_SIZE
        {%- elif field.is_enum() -%}
        1
        {%- else -%}
        std::mem::size_of::<{{ types.abi_types[field.type_name()] }}>()
        {%- endif %}
        {{- " + " if not loop.last else "" }} 
        {%- endfor %};
        {% endfor %}
        
        fn get_string(mem_base: *mut u8, offset: usize) -> Result<&'static str, String> {
            unsafe { 
                let host_ptr = mem_base.add(offset as usize);
                std::ffi::CStr::from_ptr(host_ptr as *const i8).to_str().map_err(|e| format!("UTF-8 error: {}", e))
            }
        }

        fn get_func<T>(
            caller: &mut wasmtime::Caller<'_, T>,
            func: &str,
        ) -> Result<wasmtime::Func, String> {
            #[cfg(test)]
            {
                (unsafe { super::GET_FUNCTION.as_ref().expect("Forgot to set the global GET_FUNCTION?") }(func))
                .ok_or_else(|| format!("Failed to get function `{}`.", func))
            }

            #[cfg(not(test))]
            {
                caller
                    .get_export(func)
                    .and_then(|e| e.into_func())
                    .ok_or_else(|| format!("Failed to get function `{}`.", func))
            }
        }

        fn get_memory<T>(
            caller: &mut wasmtime::Caller<'_, T>,
            mem: &str,
        ) -> Result<wasmtime::Memory, String> {
            #[cfg(test)]
            {
                (unsafe { super::GET_MEMORY.as_ref().expect("Forgot to set the global GET_MEMORY?") }(mem))
                .ok_or_else(|| format!("Failed to get memory `{}`", mem))
            }

            #[cfg(not(test))]
            {
                caller
                    .get_export(mem)
                    .and_then(|e| e.into_memory())
                    .ok_or_else(|| format!("Failed to get memory `{}`", mem))
            }
        }
     
        fn allocate<T>(caller: &mut wasmtime::Caller<'_, T>,amount: usize) -> Result<usize, String> {
            let allocator = &get_func(caller, "allocate_wasm_mem")?;
            let mut returns = [wasmtime::Val::null()];
            allocator.call(caller, &[(amount as {{ types.abi_size_type }}).into()], &mut returns)
              .map_err(|e| e.to_string())
              .and_then(|_| returns[0].{{ types.abi_size_type }}().ok_or_else(|| String::from("Allocation function returned the wrong type")))
              .map(|x| x as usize)
        }

        {% for function in functions.values() %}
        {% filter indent(width=8, first=False) %}
{% include "wrapper-function.jinja2.rs" %}
        {% endfilter %}

        {% endfor %}
    }
}
