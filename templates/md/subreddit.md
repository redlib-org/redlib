{% import "utils.md" as utils -%}
# {% if t.sub.name != "" %}r/{{ t.sub.name }}{% else %}Front page{% endif %}
{%- if t.sub.title != "" %}

**{{ t.sub.title }}**
{%- endif %}
{%- if t.sub.description != "" %}

{{ t.sub.description }}
{%- endif %}
{%- if t.sub.members.0 != "" %}

{{ t.sub.members.0 }} members · {{ t.sub.active.0 }} online
{%- endif %}

Sort: {{ t.sort.0 }}{% if t.sort.1 != "" %} ({{ t.sort.1 }}){% endif %}
{%- if t.is_filtered %}

(This community is filtered.)
{%- else if t.no_posts %}

No posts were found.
{%- else if t.all_posts_filtered %}

(All content on this page has been filtered.)
{%- else if t.all_posts_hidden_nsfw %}

All posts are hidden because they are NSFW.
{%- else -%}
{%- for post in t.posts -%}
{%- if !(post.flags.nsfw && t.prefs.show_nsfw != "on") %}

---

{% call utils::post_item(post) %}
{%- endif -%}
{%- endfor -%}
{%- endif %}
{%- if t.ends.1 != "" %}

Next page: `?after={{ t.ends.1 }}`
{%- endif %}
