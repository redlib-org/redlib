// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
// Downloads every image of a gallery post as individual files.
(function () {
    document.body.classList.add("js-enabled");

    async function downloadGallery(images, baseName) {
        const a = document.createElement("a");
        a.style.display = "none";
        document.body.appendChild(a);
        try {
            for (let i = 0; i < images.length; i++) {
                const response = await fetch(images[i]);
                const blob = await response.blob();
                const url = URL.createObjectURL(blob);
                a.href = url;
                a.download = `${baseName}_${i + 1}_${new URL(images[i], location.href).pathname.split("/").pop()}`;
                a.click();
                URL.revokeObjectURL(url);
                if (i < images.length - 1) {
                    await new Promise((resolve) => setTimeout(resolve, 100));
                }
            }
        } catch (error) {
            console.error("Error downloading gallery:", error);
            alert("Failed to download some images. Please try downloading them individually.");
        } finally {
            document.body.removeChild(a);
        }
    }

    function attachDownloadHandlers() {
        document.querySelectorAll(".download_gallery > a").forEach(function (link) {
            link.addEventListener("click", function (e) {
                e.preventDefault();
                const gallery = link.closest(".post")?.querySelector(".gallery");
                if (!gallery) return;
                const images = Array.from(gallery.querySelectorAll("figure img"), (img) => img.src);
                if (images.length === 0) return;
                downloadGallery(images, link.dataset.baseName || "redlib_gallery");
            });
        });
    }

    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", attachDownloadHandlers);
    } else {
        attachDownloadHandlers();
    }
})();
// @license-end
