// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
(function () {
    var videos = Array.prototype.slice.call(document.querySelectorAll("video[autoplay]:not(.giphy-embed):not(.post_media_gif), video.hls_autoplay"));
    if (!videos.length || !("IntersectionObserver" in window)) {
        return;
    }

    var ratios = new Map();
    var current = null;
    var released = new WeakSet();
    var autoPaused = new WeakSet();

    function play(video) {
        video.play().catch(function () {});
    }

    function pause(video) {
        autoPaused.add(video);
        video.pause();
    }

    function release(video) {
        released.add(video);
        ratios.delete(video);
        if (current === video) {
            current = null;
        }
    }

    function update() {
        var best = null;
        var bestRatio = 0.5;
        ratios.forEach(function (ratio, video) {
            if (ratio >= bestRatio) {
                best = video;
                bestRatio = ratio;
            }
        });
        if (best === current) {
            return;
        }
        if (current && !current.paused) {
            pause(current);
        }
        current = best;
        if (current) {
            play(current);
        }
    }

    var observer = new IntersectionObserver(
        function (entries) {
            entries.forEach(function (entry) {
                var video = entry.target;
                if (!released.has(video)) {
                    ratios.set(video, entry.intersectionRatio);
                } else if (!entry.isIntersecting && !video.paused) {
                    pause(video);
                }
            });
            update();
        },
        { threshold: [0, 0.25, 0.5, 0.75, 1] },
    );

    videos.forEach(function (video) {
        video.removeAttribute("autoplay");
        if (!video.paused) {
            pause(video);
        }
        // A video the user plays, pauses or unmutes is no longer autoplayed; it is still paused once out of view.
        video.addEventListener("play", function () {
            if (video !== current) release(video);
        });
        video.addEventListener("pause", function () {
            if (autoPaused.has(video)) {
                autoPaused.delete(video);
            } else {
                release(video);
            }
        });
        video.addEventListener("volumechange", function () {
            if (!video.muted) release(video);
        });
        observer.observe(video);
    });
})();
// @license-end
