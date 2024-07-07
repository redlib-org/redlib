
const codecs = {
    dash: {
        mimeType: 'application/dash+xml',
        isSupported: 'MediaSource' in window
    },
    hls: {
        mimeType: 'application/vnd.apple.mpegurl',
        isSupported: undefined
    }
}

document.addEventListener('DOMContentLoaded', () => {
    var observer = new IntersectionObserver(handleVideoIntersect, {
        rootMargin: '100px',
    });
    
    var videoElements = document.querySelectorAll(".post_media_content > video[data-dash]");

    // Check if native hls playback is supported, if so we are probably on an apple device
    var videoEl = videoElements[0];
    if (videoEl) {
        var canPlay = videoEl.canPlayType(codecs.hls.mimeType)
        // Maybe is f.e. returned by Firefox on iOS
        codecs.hls.isSupported = canPlay === 'probably' || canPlay === 'maybe';
    }

    videoElements.forEach((el) => observer.observe(el));
});

function handleVideoIntersect(entries) {
    entries.forEach((entry) => {
        var videoEl = entry.target;
        var player = videojs.getPlayer(videoEl);

        if (entry.intersectionRatio > 0) {
            if (!player) {
                initPlayer(videoEl);
            }
        } else {
            if (player) {
                player.pause();
            }
        }
    });
}

function initPlayer(videoEl, forceAutoplay = false) {
    var srcDash = videoEl.dataset.dash;
    var srcHls = videoEl.dataset.hls;
    delete videoEl.dataset.dash;
    delete videoEl.dataset.hls;
    if (!srcDash) {
        return;
    }

    const autoplay = forceAutoplay || videoEl.classList.contains('autoplay');

    if (srcHls && codecs.hls.isSupported) {
        // Try to play HLS video with native playback
        videoEl.src = srcHls;
        videoEl.controls = true;
        videoEl.addEventListener('error', (err) => {
            if (err.target.error.code === 4) { // Failed to init decoder
                codecs.hls.isSupported = false;

                // Re-init player but try to use dash instead, probably
                // canPlayType returned 'maybe' and after trying to play
                // the video it wasn't supported after all
                videoEl.dataset.dash = srcDash;
                initPlayer(videoEl, true);
            }
        });

        if (autoplay) {
            videoEl.play();
        }

        return;
    }

    player = videojs(videoEl, {
        autoplay,
        controls: true,
        controlBar: {
            children: [
                'playToggle',
                'progressControl',
                'currentTimeDisplay',
                'timeDivider',
                'durationDisplay',
                'volumePanel',
                'audioTrackButton',
                'qualitySelector',
                'playbackRateMenuButton',
                'fullscreenToggle'
            ]
        },
        html5: {
            vhs: {
                enableLowInitialPlaylist: true,
                limitRenditionByPlayerDimensions: true,
                useBandwidthFromLocalStorage: true,
            }
        },
        plugins: {
            hlsQualitySelector: {
                displayCurrentQuality: true
            }
        }
    });

    if (srcDash && codecs.dash.isSupported) {
        player.src({
            src: srcDash,
            type: codecs.dash.mimeType
        });
    }

    return player;
}
