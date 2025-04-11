// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
async function copy() {
    await navigator.clipboard.writeText(document.getElementById('bincode_str').value);
}

async function set_listener() {
    document.getElementById('copy').addEventListener('click', copy);
}

window.addEventListener('load', set_listener);
// @license-end