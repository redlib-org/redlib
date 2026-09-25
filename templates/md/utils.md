{#- Shared macros for the Markdown templates. -#}

{% macro badges(post) -%}
{%- if post.flags.nsfw %} `NSFW`{% endif -%}
{%- if post.flags.spoiler %} `Spoiler`{% endif -%}
{%- if post.flags.stickied %} `Pinned`{% endif -%}
{%- endmacro %}

{% macro meta(post) -%}
r/{{ post.community }} · u/{{ post.author.name }} · {{ post.rel_time }} · {{ post.score.0 }} points · {{ post.comments.0 }} comments
{%- if post.flair.text != "" %} · [{{ post.flair.text }}]{% endif -%}
{%- endmacro %}

{% macro media(post) -%}
{%- if post.post_type == "image" -%}
![Post image]({{ post.media.url }})
{%- else if post.post_type == "video" || post.post_type == "gif" -%}
[Video]({% if post.media.url != "" %}{{ post.media.url }}{% else %}{{ post.media.alt_url }}{% endif %})
{%- else if post.post_type == "gallery" -%}
{%- for image in post.gallery -%}
{%- if !loop.first %}

{% endif -%}
![{{ image.caption }}]({{ image.url }})
{%- if image.outbound_url != "" %} ([link]({{ image.outbound_url }})){% endif -%}
{%- endfor -%}
{%- else if post.post_type == "link" -%}
<{{ post.media.url }}>
{%- endif -%}
{%- endmacro %}

{% macro poll(post) -%}
{%- match post.poll %}
{%- when Some with (poll) %}
**Poll**: {{ poll.total_vote_count }} votes, {{ poll.voting_end_timestamp.0 }}
{%- for option in poll.poll_options %}
- {{ option.text }}
{%- match option.vote_count %}{% when Some with (n) %}: {{ n }}{% when None %}{% endmatch %}
{%- endfor %}
{%- when None %}
{%- endmatch %}
{%- endmacro %}

{#- One entry of a post listing (subreddit, user, search, duplicates). -#}
{% macro post_item(post) -%}
### [{{ post.title }}]({{ post.permalink }}){% call badges(post) %}

{% call meta(post) %}
{%- if post.post_type != "self" %}

{% call media(post) %}
{%- endif %}
{%- endmacro %}

{#- A user's comment, as listed on a user page. -#}
{% macro user_comment(post) -%}
### Comment on [{{ post.link_title }}]({{ post.permalink }}#{{ post.id }})

r/{{ post.community }} · {{ post.rel_time }} · {{ post.score.0 }} points

{{ post.body_md }}
{%- endmacro %}
