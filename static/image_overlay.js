(() => {
  const setup = () => {
    const overlay = document.getElementById("image_overlay");
    if (!overlay) {
      return;
    }

    const overlayImage = document.getElementById("image_overlay_img");
    const overlayVideo = document.getElementById("image_overlay_video");
    const closeButton = overlay.querySelector(".image_overlay_close");
    const backdrop = overlay.querySelector(".image_overlay_backdrop");
    const triggers = Array.from(document.querySelectorAll(".post_thumbnail.overlay_trigger"));

    if (!overlayImage || !overlayVideo || !closeButton || !backdrop || triggers.length === 0) {
      return;
    }

    const cache = new Map();
    let hlsPlayer = null;

    const prefetchImages = () => {
      const seen = new Set();
      triggers.forEach((trigger) => {
        const src = trigger.dataset.overlaySrc || "";
        const type = trigger.dataset.overlayType || "";
        if (type === "video" || type === "gif") {
          return;
        }
        if (!src || seen.has(src)) {
          return;
        }
        seen.add(src);
        const img = new Image();
        img.src = src;
        cache.set(src, img);
      });
    };

    const openOverlayImage = (src) => {
      if (!src) {
        return;
      }
      const cached = cache.get(src);
      overlayVideo.pause();
      overlayVideo.removeAttribute("src");
      overlayVideo.load();
      overlayVideo.classList.remove("active");
      overlayImage.classList.remove("hidden");
      overlayImage.classList.add("active");
      overlayImage.src = cached ? cached.src : src;
      overlay.classList.add("active");
      overlay.setAttribute("aria-hidden", "false");
      document.body.classList.add("overlay_open");
    };

    const openOverlayVideo = (mp4Src, hlsSrc, poster) => {
      if (!mp4Src && !hlsSrc) {
        return;
      }
      if (hlsPlayer) {
        hlsPlayer.destroy();
        hlsPlayer = null;
      }
      overlayImage.removeAttribute("src");
      overlayImage.classList.remove("active");
      overlayImage.classList.add("hidden");
      overlayVideo.removeAttribute("src");
      overlayVideo.load();
      if (hlsSrc && window.Hls && window.Hls.isSupported()) {
        hlsPlayer = new window.Hls();
        hlsPlayer.loadSource(hlsSrc);
        hlsPlayer.attachMedia(overlayVideo);
      } else if (hlsSrc && overlayVideo.canPlayType("application/vnd.apple.mpegurl")) {
        overlayVideo.src = hlsSrc;
      } else if (mp4Src) {
        overlayVideo.src = mp4Src;
      }
      overlayVideo.poster = poster || "";
      overlayVideo.classList.add("active");
      overlayVideo.load();
      overlay.classList.add("active");
      overlay.setAttribute("aria-hidden", "false");
      document.body.classList.add("overlay_open");
    };

    const closeOverlay = () => {
      overlay.classList.remove("active");
      overlay.setAttribute("aria-hidden", "true");
      document.body.classList.remove("overlay_open");
      if (hlsPlayer) {
        hlsPlayer.destroy();
        hlsPlayer = null;
      }
      overlayVideo.pause();
      overlayVideo.removeAttribute("src");
      overlayVideo.load();
      overlayVideo.classList.remove("active");
      overlayImage.classList.remove("active");
      overlayImage.classList.add("hidden");
      overlayImage.removeAttribute("src");
    };

    triggers.forEach((trigger) => {
      trigger.addEventListener("click", (event) => {
        const src = trigger.dataset.overlaySrc || "";
        const type = trigger.dataset.overlayType || "";
        const mp4Src = trigger.dataset.overlayMp4 || "";
        const hlsSrc = trigger.dataset.overlayHls || "";
        const poster = trigger.dataset.overlayPoster || "";
        if (type === "video" || type === "gif") {
          if (!mp4Src && !hlsSrc) {
            return;
          }
          event.preventDefault();
          openOverlayVideo(mp4Src, hlsSrc, poster);
          return;
        }
        if (!src) {
          return;
        }
        event.preventDefault();
        openOverlayImage(src);
      });
    });

    closeButton.addEventListener("click", closeOverlay);
    backdrop.addEventListener("click", closeOverlay);

    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && overlay.classList.contains("active")) {
        closeOverlay();
      }
    });

    window.addEventListener("load", prefetchImages);
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", setup);
  } else {
    setup();
  }
})();
