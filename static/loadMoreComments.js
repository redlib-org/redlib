// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
// Replies open as their own thread when the column they would load into is
// narrower than this share of the comments' full width, which is the screen
// width on phones and capped on desktops.
const THREAD_VIEW_MIN_WIDTH_DESKTOP = 1 / 2;
const THREAD_VIEW_MIN_WIDTH_MOBILE = 2 / 3;
// The mobile breakpoint of style.css.
const mobile = matchMedia("screen and (max-width: 800px)");

const views = new Map();
const newKey = () => `${Date.now()}-${Math.random()}`;
const pagePath = () => location.pathname + location.search;
let shownPath = pagePath();
let currentKey = history.state?.view;
if (!currentKey) {
    currentKey = newKey();
    history.replaceState({ ...history.state, view: currentKey }, "");
}

document.addEventListener("click", async (event) => {
    const link = event.target.closest("a.deeper_replies, .thread_nav a");
    if (!link || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey || event.button !== 0) return;
    event.preventDefault();
    if (link.dataset.loading) return;
    link.dataset.loading = "1";
    const label = link.textContent;
    link.textContent = "Loading…";

    try {
        await (opensThread(link) ? openThread(link.href) : loadInPlace(link));
    } catch {
        window.location.href = link.href;
    } finally {
        // Also resets the link in the comments kept for the back button.
        link.textContent = label;
        delete link.dataset.loading;
    }
});

collapseDeepReplies();
// Replies inside a collapsed comment have no width until it is opened. Every
// open comment also fires "toggle" once after parsing, hence one run per frame.
let collapsePending = false;
document.addEventListener(
    "toggle",
    (event) => {
        if (!event.target.open || collapsePending) return;
        collapsePending = true;
        requestAnimationFrame(() => {
            collapsePending = false;
            collapseDeepReplies();
        });
    },
    true,
);

window.addEventListener("popstate", async (event) => {
    if (pagePath() === shownPath) return;
    const key = event.state?.view || newKey();
    const saved = views.get(key);
    try {
        if (saved) {
            showComments(saved.comments, saved.title, key);
            window.scrollTo(0, saved.scroll);
        } else {
            const page = await fetchPage(location.href);
            showComments(commentsOf(page), page.title, key);
            scrollToTarget();
        }
    } catch {
        window.location.reload();
    }
});

function opensThread(link) {
    if (!link.matches(".deeper_replies")) return true;
    // "More comments" and links on the parent's own page load in place.
    if (!link.closest(".comment") || link.getAttribute("href") === link.dataset.load) return false;
    // The parent's replies column, which the loaded replies would fill.
    return link.hasAttribute("data-new-page") || tooNarrow(link.parentElement);
}

function tooNarrow(column) {
    const full = document.getElementById("comments").getBoundingClientRect().width;
    const min = mobile.matches ? THREAD_VIEW_MIN_WIDTH_MOBILE : THREAD_VIEW_MIN_WIDTH_DESKTOP;
    return column.getBoundingClientRect().width < full * min;
}

// Replaces replies that would be squeezed below the thread view width with a
// "More replies" link to their parent, like the server's links, which then
// opens in thread view. The removed comments are fetched again from there.
function collapseDeepReplies() {
    for (const replies of document.querySelectorAll("#comments .comment > details > .replies")) {
        if (!replies.isConnected || !replies.querySelector(":scope > .comment") || replies.querySelector(".highlighted")) continue;
        if (!replies.getClientRects().length || !tooNarrow(replies)) continue;
        const created = replies.parentElement.querySelector(":scope > summary .created");
        if (!created) continue;
        // Server links inside stand for hidden comments as well.
        let hidden = replies.querySelectorAll(".comment").length;
        for (const more of replies.querySelectorAll("a.deeper_replies")) hidden += Number(more.textContent.match(/\((\d+)\)/)?.[1] || 0);
        const link = document.createElement("a");
        link.className = "deeper_replies";
        link.href = new URL(created.href).pathname + "#thread_nav";
        link.textContent = `\u2192 More replies (${hidden})`;
        replies.replaceChildren(link);
    }
}

async function fetchPage(url) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(response.status);
    return new DOMParser().parseFromString(await response.text(), "text/html");
}

function commentsOf(page) {
    const comments = page.getElementById("comments");
    if (!comments) throw new Error("no comments");
    return comments;
}

function showComments(comments, title, key) {
    const shown = document.getElementById("comments");
    views.set(currentKey, { comments: shown, title: document.title, scroll: window.scrollY });
    shown.replaceWith(comments);
    collapseDeepReplies();
    document.title = title;
    currentKey = key;
    shownPath = pagePath();
}

function scrollToTarget() {
    const target = location.hash && document.getElementById(decodeURIComponent(location.hash.slice(1)));
    (target || document.getElementById("commentQueryForms")).scrollIntoView();
}

// Shows the comments of the linked thread page as if it had been opened.
async function openThread(url) {
    const page = await fetchPage(url);
    const comments = commentsOf(page);
    const key = newKey();
    history.scrollRestoration = "manual";
    history.pushState({ view: key }, "", url);
    showComments(comments, page.title, key);
    scrollToTarget();
}

async function loadInPlace(link) {
    const page = await fetchPage(link.dataset.load || link.href);

    // The link opens either the parent comment's thread, whose replies we
    // want, or a page with just the missing batch of comments.
    const parent = link.closest(".comment");
    const fetchedParent = parent && page.getElementById(parent.id);
    const loaded = fetchedParent
        ? [...fetchedParent.querySelector(":scope > details > .replies").children]
        : [...page.querySelectorAll(".thread")].flatMap((thread) => [...thread.children]).filter((el) => !el.matches(".thread_nav"));
    const target = (el) => el.dataset.load || el.getAttribute("href");
    const fresh = loaded.filter((el) => (el.id ? !document.getElementById(el.id) : target(el) !== target(link)));
    if (!fresh.length) throw new Error("nothing new");

    const thread = link.parentElement;
    if (thread.matches(".thread")) {
        // More top-level comments: each gets a thread of its own.
        thread.replaceWith(
            ...fresh.map((el) => {
                const wrapper = document.createElement("div");
                wrapper.className = "thread";
                wrapper.append(el);
                return wrapper;
            }),
        );
    } else {
        link.replaceWith(...fresh);
    }
    collapseDeepReplies();
}
// @license-end
