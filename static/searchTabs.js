// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
// Scrolls the search tab row so the selected tab is in view on narrow screens.
// On load, as the web font changes the tabs' widths.
window.addEventListener("load", () => {
    const tabs = document.querySelector("#listing_options.search_tabs");
    const selected = tabs?.querySelector(".selected");
    if (selected) {
        tabs.scrollLeft += selected.getBoundingClientRect().left - tabs.getBoundingClientRect().left - (tabs.clientWidth - selected.offsetWidth) / 2;
    }
});

// "Show more" links to the next page; with JavaScript, that page's results
// are added below these instead.
document.addEventListener("click", async (event) => {
    const link = event.target.closest("a#show_more");
    if (!link || event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    const results = document.querySelector(".search_results");
    if (!results) return;
    event.preventDefault();
    if (link.getAttribute("aria-busy")) return;
    link.setAttribute("aria-busy", "true");
    link.textContent = "Loading…";

    try {
        const response = await fetch(link.href);
        if (!response.ok) throw new Error(response.status);
        const page = new DOMParser().parseFromString(await response.text(), "text/html");
        const fetched = page.querySelector(".search_results");
        const items = fetched ? [...fetched.children].filter((item) => !item.id || !document.getElementById(item.id)) : [];
        const focused = document.activeElement === link;
        results.append(...items);

        const next = page.getElementById("show_more");
        if (next) {
            link.replaceWith(next);
            if (focused) next.focus({ preventScroll: true });
        } else {
            link.closest("footer").remove();
            const first = items[0]?.matches("a") ? items[0] : items[0]?.querySelector("a");
            if (focused && first) first.focus({ preventScroll: true });
        }
    } catch {
        window.location.href = link.href;
    }
});
// @license-end
