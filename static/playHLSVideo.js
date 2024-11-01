// @license http://www.gnu.org/licenses/agpl-3.0.html AGPL-3.0
(function () {
    const configElement = document.getElementById('video_quality');
    const qualitySetting = configElement.getAttribute('data-value');
    if (Hls.isSupported()) {
        var videoSources = document.querySelectorAll("video source[type='application/vnd.apple.mpegurl']");
        videoSources.forEach(function (source) {
            var playlist = source.src;

            var oldVideo = source.parentNode;
            var autoplay = oldVideo.classList.contains("hls_autoplay");

            // If HLS is supported natively then don't use hls.js
            if (oldVideo.canPlayType(source.type) === "probably") {
                if (autoplay) {
                    oldVideo.play();
                }
                return;
            }

            // Replace video with copy that will have all "source" elements removed
            var newVideo = oldVideo.cloneNode(true);
            var allSources = newVideo.querySelectorAll("source");
            allSources.forEach(function (source) {
                source.remove();
            });

            // Empty source to enable play event
            newVideo.src = "about:blank";

            oldVideo.parentNode.replaceChild(newVideo, oldVideo);

            function getIndexOfDefault(length) {
                switch (qualitySetting) {
                    case 'best':
                        return length - 1;
                    case 'medium':
                        return Math.floor(length / 2);
                    case 'worst':
                        return 0;
                    default:
                        return length - 1;
                }
            }

            function initializeHls() {
                newVideo.removeEventListener('play', initializeHls);
                var hls = new Hls({ autoStartLoad: false });
                hls.loadSource(playlist);
                hls.attachMedia(newVideo);
                hls.on(Hls.Events.MANIFEST_PARSED, function () {
                    hls.loadLevel = getIndexOfDefault(hls.levels.length);
                    var availableLevels = hls.levels.map(function(level) {
                        return {
                            height: level.height,
                            width: level.width,
                            bitrate: level.bitrate,
                        };
                    });

                    addQualitySelector(newVideo, hls, availableLevels);

                    hls.startLoad();
                    newVideo.play();
                });

                hls.on(Hls.Events.ERROR, function (event, data) {
                    var errorType = data.type;
                    var errorFatal = data.fatal;
                    if (errorFatal) {
                        switch (errorType) {
                            case Hls.ErrorType.NETWORK_ERROR:
                                hls.startLoad();
                                break;
                            case Hls.ErrorType.MEDIA_ERROR:
                                hls.recoverMediaError();
                                break;
                            default:
                                hls.destroy();
                                break;
                        }
                    }

                    console.error("HLS error", data);
                });
            }

            function addQualitySelector(videoElement, hlsInstance, availableLevels) {
                var qualitySelector = document.createElement('select');
                qualitySelector.classList.add('quality-selector');
                var defaultIndex = getIndexOfDefault(availableLevels.length);
                availableLevels.forEach(function (level, index) {
                    var option = document.createElement('option');
                    option.value = index.toString();
                    var bitrate = (level.bitrate / 1_000).toFixed(0);
                    option.text = level.height + 'p (' + bitrate + ' kbps)';
                    if (index === defaultIndex) {
                        option.selected = "selected";
                    }
                    qualitySelector.appendChild(option);
                });
                qualitySelector.selectedIndex = defaultIndex;
                qualitySelector.addEventListener('change', function () {
                    var selectedIndex = qualitySelector.selectedIndex;
                    hlsInstance.nextLevel = selectedIndex;
                    hlsInstance.startLoad();
                });

                videoElement.parentNode.appendChild(qualitySelector);
            }

            newVideo.addEventListener('play', initializeHls);

            if (autoplay) {
                newVideo.play();
            }
        });
    } else {
        var videos = document.querySelectorAll("video.hls_autoplay");
        videos.forEach(function (video) {
            video.setAttribute("autoplay", "");
        });
    }
})();
// @license-end
