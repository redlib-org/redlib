
var streamProto = {
    dash: {
        mimeType: 'application/dash+xml',
        isSupported: 'MediaSource' in window
    },
    hls: {
        mimeType: 'application/vnd.apple.mpegurl',
        isSupported: undefined
    }
};

document.addEventListener('DOMContentLoaded', function () {
    var observer = new IntersectionObserver(handleVideoIntersect, {
        rootMargin: '100px'
    });
    
    var videoElements = document.querySelectorAll(".post_media_content > video[data-dash]");

    // Check if native hls playback is supported, if so we are probably on an apple device
    var videoEl = videoElements[0];
    if (videoEl) {
        var canPlayHls = videoEl.canPlayType(streamProto.hls.mimeType)
        // Maybe is f.e. returned by Firefox on iOS
        streamProto.hls.isSupported = canPlayHls === 'probably' || canPlayHls === 'maybe';
    }

    videoElements.forEach(function (el) { observer.observe(el) });
});

function handleVideoIntersect(entries) {
    entries.forEach(function (entry) {
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

    if (srcHls && streamProto.hls.isSupported) {
        function handleHlsPlayerError(err) {
            if (err.target.error.code === 4) { // Failed to init decoder
                videoEl.removeEventListener('error', handleHlsPlayerError);
                streamProto.hls.isSupported = false;

                // Re-init player but try to use dash instead, probably
                // canPlayType returned 'maybe' and after trying to play
                // the video it wasn't supported after all
                videoEl.dataset.dash = srcDash;
                initPlayer(videoEl, true);
            }
        }

        // Try to play HLS video with native playback
        videoEl.src = srcHls;
        videoEl.controls = true;
        videoEl.addEventListener('error', handleHlsPlayerError);

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

    if (srcDash && streamProto.dash.isSupported) {
        player.src({
            src: srcDash,
            type: streamProto.dash.mimeType
        });
    }

    return player;
}
