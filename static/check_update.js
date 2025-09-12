// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
async function checkInstanceUpdateStatus() {
    try {
        const response = await fetch('/commits.atom');
        const text = await response.text();
        const parser = new DOMParser();
        const xmlDoc = parser.parseFromString(text, "application/xml");
        const entries = xmlDoc.getElementsByTagName('entry');
        const localCommit = document.getElementById('git_commit').dataset.value;

        let statusMessage = '';

        if (entries.length > 0) {
            const commitHashes = Array.from(entries).map(entry => {
                const id = entry.getElementsByTagName('id')[0].textContent;
                return id.split('/').pop();
            });

            const commitIndex = commitHashes.indexOf(localCommit);

            if (commitIndex === 0) {
                statusMessage = '✅ Instance is up to date.';
            } else if (commitIndex > 0) {
                statusMessage = `⚠️ This instance is not up to date and is ${commitIndex} commits old. Test and confirm on an up-to-date instance before reporting.`;
                document.getElementById('error-446').remove();
            } else {
                statusMessage = `⚠️ This instance is not up to date and is at least ${commitHashes.length} commits old. Test and confirm on an up-to-date instance before reporting.`;
                document.getElementById('error-446').remove();
            }
        } else {
            statusMessage = '⚠️ Unable to fetch commit information.';
        }

        document.getElementById('update-status').innerText = statusMessage;
    } catch (error) {
        console.error('Error fetching commits:', error);
        document.getElementById('update-status').innerText = '⚠️ Error checking update status: ' + error;
    }
}

async function checkOtherInstances() {
    try {
        const response = await fetch('/instances.json');
        const data = await response.json();
        const instances = window.location.host.endsWith('.onion') ? data.instances.filter(i => i.onion) : data.instances.filter(i => i.url);
        if (instances.length == 0) return;
        const randomInstance = instances[Math.floor(Math.random() * instances.length)];
        const instanceUrl = randomInstance.url ?? randomInstance.onion;
        // Set the href of the <a> tag to the instance URL with path included
        document.getElementById('random-instance').href = instanceUrl + window.location.pathname;
        document.getElementById('random-instance').innerText = "Visit Random Instance";
    } catch (error) {
        console.error('Error fetching instances:', error);
        document.getElementById('update-status').innerText = '⚠️ Error checking other instances: ' + error;
    }
}

// Set the target URL when the page loads
window.addEventListener('load', checkOtherInstances);

checkInstanceUpdateStatus();
// @license-end