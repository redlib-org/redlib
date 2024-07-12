
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

var players = {}

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
        var player = videoEl._dashjs_player;

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

    var autoplay = forceAutoplay || videoEl.classList.contains('autoplay');

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

    // remove mp4 source on video element
    videoEl.querySelectorAll('source').forEach(function (el) { el.remove(); });
    delete videoEl.src;
    delete videoEl.srcObject;
    videoEl.load(); // required to really remove the source without replacing the video element

    var player = dashjs.MediaPlayer().create();
    player.updateSettings({
        streaming: {
            abr: {
                autoSwitchBitrate: true,
                limitBitrateByPortal: true,
                usePixelRatioInLimitBitrateByPortal: true
            },
            buffer: {
                fastSwitchEnabled: true
            },
            scheduling: {
                scheduleWhilePaused: false
            }
        }
    });

    player.on(dashjs.MediaPlayer.events.STREAM_ACTIVATED, function (e) { onStreamActivated(e, player); });
    player.on(dashjs.MediaPlayer.events.QUALITY_CHANGE_RENDERED, function (e) { onQualityChangeRendered(e, player); });
    player.initialize(videoEl, srcDash, autoplay);

    if (!videoEl._dashjs_player) {
        videoEl._dashjs_player = player;
    }

    return player;
}

function onStreamActivated(e, player) {
    /* e = { streamInfo: { ... } } */
    if (player.getBitrateInfoListFor) {
        var videoBitrates = player.getBitrateInfoListFor('video');
        addQualitySelector(player, videoBitrates);
    }
}

function onQualityChangeRendered(e, player) {
    /* e = { mediaType: "video", oldQuality: 1, newQuality: 2, streamId: "0", type: "qualityChangeRendered" } */
    var videoEl = player.getVideoElement();
    var qualitySelector = videoEl.nextElementSibling;
    if (qualitySelector && qualitySelector.tagName === 'SELECT') {
        var selectedIndex = qualitySelector.selectedIndex;
        var optionCount = qualitySelector.options.length;

        if (selectedIndex === optionCount - 1) { // auto quality
            var [mode] = qualitySelector.options[e.newQuality].innerText.split(' ', 2);
            qualitySelector.options[selectedIndex].innerText = 'auto ('+ mode +')';
        } else {
            qualitySelector.selectedIndex = e.newQuality;
        }
    }
}

function addOption(selectEl, value, label, isSelected = false) {
    var option = document.createElement('option');
    option.value = value;
    option.text = label;
    if (isSelected) {
        option.selected = "selected"
    }
    selectEl.appendChild(option);
}

function addQualitySelector(player, availableLevels) {
    var qualitySelector = document.createElement('select');
    qualitySelector.classList.add('quality-selector');

    availableLevels.forEach(function (level, index) {
        var bitrate = (level.bitrate / 1e3).toFixed(0);
        var label = level.height + 'p (' + bitrate + ' kbps)';

        addOption(qualitySelector, index.toString(), label);
    });
    addOption(qualitySelector, 'auto', 'auto', true);

    var lastIndex = availableLevels.length;
    qualitySelector.selectedIndex = lastIndex;
    qualitySelector.addEventListener('change', function () {
        var selectedIndex = qualitySelector.selectedIndex;

        // Only auto switch bitrate if user did not manually change it
        var autoSwitchBitrate = selectedIndex >= lastIndex ? true : false;
        player.updateSettings({
            streaming: {
                abr: {
                    autoSwitchBitrate,
                }
            }
        });

        if (!autoSwitchBitrate) {
            player.setQualityFor('video', selectedIndex, true);
        }
    });

    var videoEl = player.getVideoElement();
    videoEl.parentNode.appendChild(qualitySelector);
}
