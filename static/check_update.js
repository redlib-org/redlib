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
                document.getElementById('error-318').remove();
            } else {
                statusMessage = `⚠️ This instance is not up to date and is at least ${commitHashes.length} commits old. Test and confirm on an up-to-date instance before reporting.`;
                document.getElementById('error-318').remove();
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
        const instancesResponse = await fetch('/instances.json');
        const instancesData = await instancesResponse.json();
        const randomInstance = instancesData.instances[Math.floor(Math.random() * instancesData.instances.length)];
        const instanceUrl = randomInstance.url;
        
        // Fetch current user settings to transfer them to the new instance
        let targetUrl = instanceUrl + window.location.pathname;
        
        try {
            const settingsResponse = await fetch('/settings.json');
            if (settingsResponse.ok) {
                const settingsData = await settingsResponse.json();
                // Check if settings were successfully encoded and are available for transfer
                if (settingsData.success && settingsData.url_encoded) {
                    targetUrl = instanceUrl + '/settings/restore/?' + settingsData.url_encoded + '&redirect=' + encodeURIComponent(window.location.pathname.substring(1));
                } else if (settingsData.error) {
                    console.warn('Settings server error:', settingsData.error, '- visiting random instance without settings transfer');
                } else {
                    console.warn('Settings encoding failed - visiting random instance without settings transfer');
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
        document.getElementById('random-instance').innerText = "Visit Random Instance";
    } catch (error) {
        console.error('Error fetching instances:', error);
        document.getElementById('update-status').innerText = '⚠️ Error checking other instances: ' + error;
    }
}

// Set the target URL when the page loads
window.addEventListener('load', checkOtherInstances);

checkInstanceUpdateStatus();
