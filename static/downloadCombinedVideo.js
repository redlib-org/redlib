// Download combined video and audio using ffmpeg.wasm
// Usage: downloadCombinedVideo(videoUrl, audioUrl, downloadName)

(function() {
    const { FFmpeg } = FFmpegWASM;
    const { fetchFile } = FFmpegUtil;

    // The ffmpeg core is a ~32MB wasm module. Loading it dominates the wait,
    // so load it at most once and reuse the same instance for every download.
    let ffmpegPromise = null;
    function getFFmpeg(onStatus) {
        if (!ffmpegPromise) {
            const ffmpeg = new FFmpeg();
            ffmpegPromise = ffmpeg
                .load({
                    coreURL: '/static/ffmpeg/ffmpeg-core.js',
                    wasmURL: '/static/ffmpeg/ffmpeg-core.wasm',
                })
                .then(function() { return ffmpeg; })
                .catch(function(err) {
                    // Allow a later attempt to retry from scratch
                    ffmpegPromise = null;
                    throw err;
                });
        } else if (onStatus) {
            // Already loading/loaded from an earlier warm-up; reflect that
            onStatus('loading');
        }
        return ffmpegPromise;
    }

    // Kick off loading the core ahead of time (e.g. on hover) so it is ready
    // by the time the user actually clicks. Errors are swallowed here; the
    // real download path surfaces them.
    function warmFFmpeg() {
        getFFmpeg().catch(function() {});
    }

    // ffmpeg.wasm runs one job at a time on a shared instance, so serialize
    // muxing operations through this chain.
    let queue = Promise.resolve();

    window.downloadCombinedVideo = function(videoUrl, audioUrl, downloadName, onStatus) {
        const status = typeof onStatus === 'function' ? onStatus : function() {};

        const run = queue.then(async function() {
            status('loading');
            const ffmpeg = await getFFmpeg();

            // Fetch the separate files and write them to the virtual filesystem
            status('downloading');
            await ffmpeg.writeFile('input_video.mp4', await fetchFile(videoUrl));
            await ffmpeg.writeFile('input_audio.mp4', await fetchFile(audioUrl));

            // Execute the muxing command (Copy codecs for speed)
            status('merging');
            await ffmpeg.exec([
                '-i', 'input_video.mp4',
                '-i', 'input_audio.mp4',
                '-c:v', 'copy',
                '-c:a', 'copy',
                '-map', '0:v:0',
                '-map', '1:a:0',
                'output.mp4'
            ]);

            // Read the result and trigger a browser download
            const data = await ffmpeg.readFile('output.mp4');
            const url = URL.createObjectURL(new Blob([data.buffer], { type: 'video/mp4' }));

            const a = document.createElement('a');
            a.href = url;
            a.download = downloadName || 'combined_video.mp4';
            a.click();

            // Cleanup the output so it does not linger in the virtual FS
            URL.revokeObjectURL(url);
            try { await ffmpeg.deleteFile('output.mp4'); } catch (e) { /* ignore */ }
            status('done');
        });

        run.then(function() { status('done'); }, function() { status('error'); });

        // Keep the queue alive regardless of this job's outcome
        queue = run.catch(function() {});
        return run;
    };

    // Human-readable label shown in the indicator for each stage
    const STATUS_LABELS = {
        loading: 'loading…',
        downloading: 'downloading…',
        merging: 'merging…',
        done: 'done ✓',
        error: 'failed ✗',
    };

    // Attach click handlers to combined download links
    function attachDownloadHandlers() {
        document.querySelectorAll('a.combined_download').forEach(function(link) {
            // Indicator span lives right after the link in the template
            const indicator = link.parentElement.querySelector('.download_status');

            function setStatus(stage) {
                if (!indicator) return;
                indicator.textContent = STATUS_LABELS[stage] || '';
                indicator.dataset.stage = stage;
                indicator.hidden = false;
            }

            // Start loading the core as soon as the user shows intent, so the
            // expensive load overlaps with their decision to click. Fires once.
            ['pointerenter', 'focus', 'pointerdown'].forEach(function(evt) {
                link.addEventListener(evt, warmFFmpeg, { once: true, passive: true });
            });

            link.addEventListener('click', function(e) {
                e.preventDefault();
                // Ignore extra clicks while a download is already in progress
                if (link.dataset.busy === 'true') return;
                link.dataset.busy = 'true';

                const videoUrl = link.getAttribute('data-video');
                const audioUrl = link.getAttribute('data-audio');
                const downloadName = link.getAttribute('data-name');

                downloadCombinedVideo(videoUrl, audioUrl, downloadName, setStatus)
                    .catch(function(err) {
                        console.error('Combined download failed:', err);
                    })
                    .finally(function() {
                        link.dataset.busy = 'false';
                    });
            });
        });
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', attachDownloadHandlers);
    } else {
        attachDownloadHandlers();
    }
})();
