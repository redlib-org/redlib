// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
// Previews the picked themes before they're saved; all themes are already in style.css
const themes = [...document.querySelectorAll("#theme option")].map((option) => option.value);
function previewTheme() {
    const body = document.body.classList;
    for (const theme of themes) body.remove(theme, `light-${theme}`, `dark-${theme}`);
    const theme = document.getElementById("theme").value;
    if (theme !== "system") return body.add(theme);
    for (const scheme of ["light", "dark"]) {
        const picked = document.getElementById(`${scheme}_theme`).value;
        if (picked) body.add(`${scheme}-${picked}`);
    }
}
for (const id of ["theme", "light_theme", "dark_theme"]) document.getElementById(id).addEventListener("change", previewTheme);
// @license-end
