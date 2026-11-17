// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
// Shows a red bar above the footer when this instance is behind its source repository.
async function checkInstanceUpdateStatus() {
    try {
        const response = await fetch("/commits.atom");
        const text = await response.text();
        const xmlDoc = new DOMParser().parseFromString(text, "application/xml");
        const entries = xmlDoc.getElementsByTagName("entry");
        if (entries.length === 0) return;

        const commitHashes = Array.from(entries).map((entry) => entry.getElementsByTagName("id")[0].textContent.split("/").pop());
        const commitIndex = commitHashes.indexOf(document.getElementById("git_commit").dataset.value);
        if (commitIndex === 0) return;

        const behind = commitIndex > 0 ? `${commitIndex}` : `at least ${commitHashes.length}`;
        const bar = document.getElementById("update-status");
        bar.innerText = `⚠️ This instance is ${behind} commits behind. Confirm on an up-to-date instance before reporting.`;
        // Second footer row, directly above the normal one.
        const footer = document.querySelector("body > footer");
        footer?.prepend(bar);
        footer?.classList.add("has_update");
        bar.hidden = false;
        document.body.classList.add("update_footer_shown");
    } catch (error) {
        console.error("Error fetching commits:", error);
    }
}

async function checkOtherInstances() {
    try {
        const response = await fetch("/instances.json");
        const data = await response.json();
        const instances = window.location.host.endsWith(".onion") ? data.instances.filter((i) => i.onion) : data.instances.filter((i) => i.url);
        if (instances.length == 0) return;
        const randomInstance = instances[Math.floor(Math.random() * instances.length)];
        const instanceUrl = randomInstance.url ?? randomInstance.onion;
        // Set the href of the <a> tag to the instance URL with path included
        const link = document.getElementById("random-instance");
        link.href = instanceUrl + window.location.pathname;
        link.innerText = "Try another instance";
    } catch (error) {
        console.error("Error fetching instances:", error);
    }
}

// Set the target URL when the page loads
window.addEventListener("load", checkOtherInstances);

checkInstanceUpdateStatus();
// @license-end
