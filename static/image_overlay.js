(() => {
  const setup = () => {
    const overlay = document.getElementById("image_overlay");
    if (!overlay) {
      return;
    }

    const overlayImage = document.getElementById("image_overlay_img");
    const closeButton = overlay.querySelector(".image_overlay_close");
    const backdrop = overlay.querySelector(".image_overlay_backdrop");
    const triggers = Array.from(document.querySelectorAll(".post_thumbnail.overlay_trigger"));

    if (!overlayImage || !closeButton || !backdrop || triggers.length === 0) {
      return;
    }

    const cache = new Map();

    const prefetchImages = () => {
      const seen = new Set();
      triggers.forEach((trigger) => {
        const src = trigger.dataset.overlaySrc || "";
        if (!src || seen.has(src)) {
          return;
        }
        seen.add(src);
        const img = new Image();
        img.src = src;
        cache.set(src, img);
      });
    };

    const openOverlay = (src) => {
      if (!src) {
        return;
      }
      const cached = cache.get(src);
      overlayImage.src = cached ? cached.src : src;
      overlay.classList.add("active");
      overlay.setAttribute("aria-hidden", "false");
      document.body.classList.add("overlay_open");
    };

    const closeOverlay = () => {
      overlay.classList.remove("active");
      overlay.setAttribute("aria-hidden", "true");
      document.body.classList.remove("overlay_open");
    };

    triggers.forEach((trigger) => {
      trigger.addEventListener("click", (event) => {
        const src = trigger.dataset.overlaySrc || "";
        if (!src) {
          return;
        }
        event.preventDefault();
        openOverlay(src);
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
