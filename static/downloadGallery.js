// Usage: downloadGallery(images, baseName)

(function() {
    // Add js-enabled class for CSS detection
    document.body.classList.add('js-enabled');

    window.downloadGallery = async function(images, baseName) {
        // Request permission to download multiple files
        // This will show a browser prompt asking for permission
        try {
            // Create a single reusable download link
            const a = document.createElement('a');
            a.style.display = 'none';
            document.body.appendChild(a);

            // Use the File System Access API if available, otherwise fall back to individual downloads
            for (let i = 0; i < images.length; i++) {
                const imageUrl = images[i];
                const filename = `${baseName}_${i + 1}.jpg`;

                // Fetch the image as a blob
                const response = await fetch(imageUrl);
                const blob = await response.blob();

                // Reuse the same link element
                const url = URL.createObjectURL(blob);
                a.href = url;
                a.download = filename;
                a.click();
                URL.revokeObjectURL(url);

                // Small delay between downloads to avoid overwhelming the browser
                if (i < images.length - 1) {
                    await new Promise(resolve => setTimeout(resolve, 100));
                }
            }

            document.body.removeChild(a);
        } catch (error) {
            console.error('Error downloading gallery:', error);
            alert('Failed to download some images. Please try downloading them individually.');
        }
    };

    // Attach click handlers to gallery download links
    function attachDownloadHandlers() {
        document.querySelectorAll('.download_gallery > a').forEach(function(link) {
            link.addEventListener('click', function(e) {
                e.preventDefault();

                // Find all gallery images
                const post = link.closest('.post');
                if (!post) return;

                const gallery = post.querySelector('.gallery');
                if (!gallery) return;

                const images = [];
                gallery.querySelectorAll('figure img').forEach(function(img) {
                    images.push(img.src);
                });

                if (images.length === 0) return;

                // Get base name from data attribute
                const baseName = link.getAttribute('data-base-name') || 'redlib_gallery';

                downloadGallery(images, baseName);
            });
        });
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', attachDownloadHandlers);
    } else {
        attachDownloadHandlers();
    }
})();
