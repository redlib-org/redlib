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
            } else {
                statusMessage = `⚠️ This instance is not up to date and is at least ${commitHashes.length} commits old. Test and confirm on an up-to-date instance before reporting.`;
            }
        } else {
            statusMessage = '⚠️ Unable to fetch commit information.';
        }

        document.getElementById('update-status').innerText = statusMessage;
    } catch (error) {
        console.error('Error fetching commits:', error);
        document.getElementById('update-status').innerText = '⚠️ Error checking update status.';
    }
}

checkInstanceUpdateStatus();
