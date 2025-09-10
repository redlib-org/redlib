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
        // Fetch list of available instances
        const response = await fetch('/instances.json');
        const data = await response.json();
        const instances = window.location.host.endsWith('.onion') ? data.instances.filter(i => i.onion) : data.instances.filter(i => i.url);
        if (instances.length == 0) return;
        const randomInstance = instances[Math.floor(Math.random() * instances.length)];
        const instanceUrl = randomInstance.url ?? randomInstance.onion;
        
        // Fetch current user settings to transfer them to the new instance
        let targetUrl = instanceUrl + window.location.pathname;
        let text = "Visit Random Instance";
        
        try {
            const settingsResponse = await fetch('/settings.json');
            if (settingsResponse.ok) {
                const urlEncoded = await settingsResponse.text();
                if (urlEncoded && urlEncoded.trim()) {
                    targetUrl = instanceUrl + '/settings/restore/?' + urlEncoded + '&redirect=' + encodeURIComponent(window.location.pathname.substring(1));
                    text += " (bringing preferences)";
                } else {
                    console.warn('Settings encoding returned empty - visiting random instance without settings transfer');
                }
            } else {
                console.warn('Could not fetch user settings (HTTP', settingsResponse.status + ') - visiting random instance without settings transfer');
            }
        } catch (settingsError) {
            console.warn('Error fetching user settings:', settingsError);
            console.warn('Visiting random instance without settings transfer');
        }
        
        // Set the href of the <a> tag to the instance URL with path and settings included
        document.getElementById('random-instance').href = targetUrl;
        document.getElementById('random-instance').innerText = text;
    } catch (error) {
        console.error('Error fetching instances:', error);
        document.getElementById('update-status').innerText = '⚠️ Error checking other instances: ' + error;
    }
}

// Set the target URL when the page loads
window.addEventListener('load', checkOtherInstances);

checkInstanceUpdateStatus();
