{% if function.doc_string %}
{% for doc in function.doc_string.splitlines() %}
/// {{doc}}
{% endfor %}
{% endif %}
fn {{ function.name | snakecase }}<'input>(
    &mut self, 
    {% for arg in function.arguments.values() %}
    {{ types.trait_input(arg, lifetime_name="input") }},
    {% endfor %}
) -> Result<{{ types.trait_output(function) }}, String>;

