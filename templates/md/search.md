{% import "utils.md" as utils -%}
# Search: {{ t.params.q }}
{%- if t.sub != "" %}

In r/{{ t.sub }}
{%- endif %}

Sort: {{ t.params.sort }}{% if t.params.t != "" %} ({{ t.params.t }}){% endif %}
{%- if t.subreddits.len() > 0 %}

## Communities
{% for sub in t.subreddits %}
- r/{{ sub.name }}{% if sub.description != "" %}: {{ sub.one_line_description() }}{% endif %} ({{ sub.subscribers.0 }} members)
{%- endfor %}
{%- endif %}

## Posts
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
{%- if t.params.after != "" %}

Next page: `?after={{ t.params.after }}`
{%- endif %}
