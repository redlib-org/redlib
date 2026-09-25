{% import "utils.md" as utils -%}
# Duplicates of: {{ t.post.title }}

{% call utils::meta(t.post) %}

Original: {{ t.post.permalink }}
{%- if t.post.num_duplicates == 0 %}

No duplicates found.
{%- else if t.all_posts_filtered %}

(All duplicates have been filtered.)
{%- else -%}
{%- for post in t.duplicates -%}
{%- if !(post.flags.nsfw && t.prefs.show_nsfw != "on") %}

---

{% call utils::post_item(post) %}
{%- endif -%}
{%- endfor -%}
{%- endif %}
{%- if t.params.after != "" %}

Next page: `?after={{ t.params.after }}`
{%- endif %}
