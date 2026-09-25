{%- if c.kind == "more" -%}
- *{{ c.more_count }} more replies*
{%- else if c.kind == "t1" -%}
- **u/{{ c.author.name }}**{% if c.author.name == c.post_author %} (OP){% endif %} · {{ c.score.0 }} points · {{ c.rel_time }}
{%- if c.edited.0 != "" %} · edited {{ c.edited.0 }}{% endif %}
{%- filter indent(2, true) %}

{% if c.is_filtered -%}
(Filtered content)
{%- else -%}
{{ c.body_md }}
{%- endif %}
{%- for r in c.replies %}

{{ r.markdown() }}
{%- endfor %}
{%- endfilter %}
{%- endif -%}
