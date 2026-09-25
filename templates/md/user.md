{% import "utils.md" as utils -%}
# u/{{ t.user.name }}
{%- if t.user.title != "" %}

**{{ t.user.title }}**
{%- endif %}
{%- if t.user.description != "" %}

{{ t.user.description }}
{%- endif %}

{{ t.user.karma }} karma · joined {{ t.user.created }}

Listing: {{ t.listing }}{% if t.sort.0 != "" %} · Sort: {{ t.sort.0 }}{% if t.sort.1 != "" %} ({{ t.sort.1 }}){% endif %}{% endif %}
{%- if t.is_filtered %}

(This user is filtered.)
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

{% if post.title != "" -%}
{% call utils::post_item(post) %}
{%- else -%}
{% call utils::user_comment(post) %}
{%- endif %}
{%- endif -%}
{%- endfor -%}
{%- endif %}
{%- if t.ends.1 != "" %}

Next page: `?after={{ t.ends.1 }}`
{%- endif %}
