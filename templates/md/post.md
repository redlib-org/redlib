{% import "utils.md" as utils -%}
# {{ t.post.title }}{% call utils::badges(t.post) %}

{% call utils::meta(t.post) %}

Permalink: {{ t.post.permalink }}
{%- if t.post.post_type != "self" %}

{% call utils::media(t.post) %}
{%- endif %}
{%- if t.post.body_md != "" %}

{{ t.post.body_md }}
{%- endif %}
{%- match t.post.poll %}{% when Some with (_p) %}

{% call utils::poll(t.post) %}
{%- when None %}{% endmatch %}

---

## Comments ({{ t.post.comments.0 }}{% if t.sort != "" %}, sorted by {{ t.sort }}{% endif %})
{%- if t.comment_query != "" %}

Only comments containing "{{ t.comment_query }}".
{%- endif %}
{%- if t.single_thread %}

Showing a single comment thread. All comments: {{ t.post.permalink }}
{%- endif %}
{%- for c in t.comments %}

{{ c.markdown() }}
{%- endfor %}
