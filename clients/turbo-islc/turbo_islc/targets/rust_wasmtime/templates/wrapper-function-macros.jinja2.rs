{% macro list_input(arg, in_record=False) %}
{% set arg_snake = arg.name | snakecase %}
{
    {% set arg_snake = arg.name | snakecase %}
    {% if arg.is_record() %}
    {% set arg_type_out = "super::%s" % arg.type_name() | camelcase %}
    {% set vec_len %}{{ arg_snake }}_len{% endset %}
    {# TODO: Check if this works for list of structs that has lifetimes inside #}
    let mut vector: Vec<{{arg_type_out}}> = Vec::with_capacity({{vec_len}} as usize);
    let array_offset = {{ arg_snake }} as usize;

    for i in 0..({{vec_len}} as usize) {
        let {{ arg_snake }} = array_offset + i * {{ arg.type_name() | snakecase | upper}}_SIZE;
        vector.push({{ convert_input(arg, is_list_override=True, ref=False) | indent(4)}});
    }
    {% else %}
    {% set arg_type, arg_type_out = (types.abi_types[arg.type_name()], types.rust_types[arg.type_name()]) %}
    {% set vec_len %}{{ arg_snake }}_len{% endset %}
    let mut vector: Vec<{{arg_type_out}}> = Vec::with_capacity({{vec_len}} as usize);
    let array_ptr = unsafe { mem_base.add({{ arg_snake }} as usize) as *const {{arg_type}} };
    let slice = unsafe { std::slice::from_raw_parts(array_ptr, {{ vec_len }} as usize) };

    slice.iter().for_each(|{{ arg_snake }}| {
        vector.push(*{{ convert_input(arg, is_list_override=True, ref=False) | indent(4)}});
    });
    {% endif %}
    vector
}
{%- endmacro %}

{% macro field_offset(fields) %}
{% for field in fields %}
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
{%- endfor %}
{% endmacro %}

{% macro add_field_offset(fields_processed) %}
{% set field_offset_value = field_offset(fields_processed) %}
{{- (" + %s" % field_offset_value) if field_offset_value else "" }}
{%- endmacro %}

{% macro record_input(arg) %}
{
    {% set record = arg.as_record() %}
    {% set arg_snake = arg.name | snakecase %}
    let record_base = unsafe {
        mem_base.add({{ arg_snake }} as usize) as *const u8
    };

    super::{{ record.name | camelcase }} {
    {% set fields_processed = [] %}
    {% for (name, field) in record.fields.items() %}
        {% set name_snake = name | snakecase %}
        {{ name_snake }}: {% if field.is_reference() %}&{% endif -%}
    {% if field.is_record() and not field.is_list() -%}
    {
            let {{ name_snake }} = {{arg_snake}} as usize{{ add_field_offset(fields_processed) }};
            {{ convert_input(field, ref=False) | indent(12) }}
        },
    {% else -%}
    {
            let value = unsafe { record_base.add({{ field_offset(fields_processed) | default("0", true) }}) };
        {% if field.is_list() %}
            let {{ name_snake }} = unsafe { *(value as *const {{ types.abi_size_type }}) };
            let {{ name_snake }}_len = {
                let value = unsafe { record_base.add(std::mem::size_of::<{{ types.abi_size_type }}>() {{ add_field_offset(fields_processed) }}) };
                unsafe { *(value as *const {{ types.abi_size_type }}) }
            };
            {{ convert_input(field, ref=False) | indent(12) }}
        {% elif field.is_enum() %}
            let {{ name_snake }} = unsafe { *(value as *const u8) };
            {{ convert_input(field, ref=False) | indent(12) }}
        {% elif field.type_name() == "string" %}
            let {{ name_snake }} = unsafe { *(value as *const {{ types.abi_size_type }}) };
            {{ convert_input(field, ref=False) | indent(12) }}
        {% else %}
            unsafe { *(value as *const {{types.abi_types[field.type_name()]}}) }
        {% endif %}
        },
    {% endif %}
    {% set fields_processed = fields_processed.append(field) %}
    {% endfor %}
    }
}
{%- endmacro %}

{% macro convert_input(arg, is_list_override=False, ref=True) %}
{% if arg.is_list() and not is_list_override %}
{% if ref %}&{% endif %}{{ list_input(arg) }}
{%- elif arg.is_record() %}
{% if ref %}&{% endif %}{{ record_input(arg) }}
{%- elif arg.is_enum() %}
try_or_errmsg!(caller, super::{{ arg.type_name() | camelcase }}::try_from({{ arg.name |snakecase}} as u8))
{%- elif arg.type_name() == "string" %}
try_or_errmsg!(caller, get_string(mem_base, {{ arg.name | snakecase }} as usize)){%if not ref %}.to_string(){% endif %}
{% else %}
{{ arg.name | snakecase }}{% if arg.type_name() == "bool" %} == 1{% endif %}
{%- endif %}
{% endmacro %}

{% macro add_multi_result(arg_name, multi=False)%}
{% if multi %}.{{ arg_name }}{% endif %}
{% endmacro %}

{% macro convert_output(arg, trait_result, is_list_override=False, multi=False) %}
{% set arg_snake = arg.name | snakecase %}
{% set arg_snake_out = arg_snake + "_out" %}
{% set arg_snake_out_value = arg_snake_out + "_value" %}
{% if arg.is_list() and not is_list_override %}
{% set arg_size %}
{% if arg.type_name() in types.abi_types -%}
std::mem::size_of::<{{types.abi_types[arg.type_name()]}}>()
{%- else %}
{{ arg.type_name() | snakecase | upper }}_SIZE
{%- endif -%}
{% endset %}
let offset = try_or_errmsg!(caller, allocate(&mut caller, {{ trait_result }}.len() * {{ arg_size }}));

for (i, item) in {{trait_result}}.iter().enumerate() {
    let {{ arg_snake_out }} = (i * {{ arg_size }}) + offset;
    {{ convert_output(arg, "item", is_list_override=True) | indent(4) }}
}
unsafe {
    *(mem_base.add( {{ arg_snake_out }} as usize ) as *mut {{ types.abi_size_type }}) = offset as {{ types.abi_size_type }};
    *(mem_base.add( {{ arg_snake_out }}_len as usize ) as *mut {{ types.abi_size_type }}) = {{trait_result}}.len() as {{ types.abi_size_type }};
};
{%- elif arg.type_name() == "string" %}
unsafe { *(mem_base.add( {{ arg_snake_out }} as usize ) as *mut {{ types.abi_size_type }}) = try_or_errmsg!(
    caller,
    create_wasm_string(&mut caller, &{{ trait_result }}{{ add_multi_result(arg_snake, multi) }})
) as {{ types.abi_size_type }}
};
{%- elif arg.is_enum() %}
//Write enum {{ arg.name | camelcase }}
unsafe { *(mem_base.add( {{ arg_snake_out }} as usize ) as *mut u8) = {{ trait_result }}{{ add_multi_result(arg_snake, multi) }} as u8};
{%- elif arg.is_record() %}
// Write record {{ arg.name | camelcase }}
{
    let record_base = unsafe {mem_base.add({{ arg_snake_out }} as usize) as *mut u8};
    {% set processed_fields = [] %}
    {% for field in arg.as_record().fields.values() %}
    {% set field_snake = field.name | snakecase %}
    // Write field {{ arg.name | camelcase }}.{{ field_snake }}
    {% set result_field = "%s.%s" % (trait_result, field_snake) %}
    {
        let {{ field.name | snakecase }}_out = {{arg_snake_out}} as usize {{ add_field_offset(processed_fields) -}};
        {% if field.is_list() %}
        let {{ field.name | snakecase }}_out_len = {{arg_snake_out}} as usize {{ add_field_offset(processed_fields) }} + std::mem::size_of::<{{ types.abi_size_type }}>();
        {% endif %}
        {{ convert_output(field, result_field) | indent(8) }}
    };
    {% set processed_fields = processed_fields.append(field) %}
    {% endfor %}
}
{% else  %}
unsafe { *(mem_base.add( {{ arg_snake_out }} as usize ) as *mut {{ types.abi_types[arg.type_name()] }}) = {% if arg.type_name() == "bool" %}({% endif %}
    {%- if is_list_override %}*{% endif %}{{trait_result}}{{ add_multi_result(arg_snake, multi) }}{% if arg.type_name() == "bool" %}).into(){% endif %} };
{%- endif %}
{% endmacro %}
