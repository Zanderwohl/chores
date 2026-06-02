(function() {
    const photos = window.SLIDESHOW_PHOTOS || [];
    const PRELOAD_COUNT = 3;
    
    // Timing configuration
    const DISPLAY_DURATION = 8000;           // ms to show each slide
    const TRANSITION_DURATION = 1500;        // ms for auto-advance crossfade
    const MANUAL_TRANSITION_DURATION = 1000; // ms for arrow key transitions
    
    // State
    let currentIndex = 0;
    let canvas = null;
    let ctx = null;
    let offscreenCanvas = null;
    let offscreenCtx = null;
    let currentCanvas = null;  // Stores the current frame for blending
    let currentCtx = null;
    const preloadedImages = new Map();
    
    // Timer state
    let displayTimerId = null;
    let transitionAnimationId = null;
    let isTransitioning = false;
    let transitionTargetIndex = null;
    
    // Blend function - swappable closure for different transition effects
    const blendFn = function(mainCtx, currentCanvas, nextCanvas, progress) {
        // Simple crossfade using globalAlpha
        mainCtx.globalAlpha = 1;
        mainCtx.drawImage(currentCanvas, 0, 0);
        mainCtx.globalAlpha = progress;
        mainCtx.drawImage(nextCanvas, 0, 0);
        mainCtx.globalAlpha = 1;
    };
    
    function init() {
        if (photos.length === 0) {
            return;
        }
        
        const container = document.getElementById('idle-content');
        if (!container) return;
        
        container.style.cssText = `
            position: fixed;
            inset: 0;
            display: flex;
            align-items: center;
            justify-content: center;
            background: #000;
            cursor: pointer;
        `;
        
        // Main display canvas
        canvas = document.createElement('canvas');
        canvas.width = window.innerWidth;
        canvas.height = window.innerHeight;
        canvas.style.cssText = `
            width: 100%;
            height: 100%;
        `;
        container.appendChild(canvas);
        ctx = canvas.getContext('2d');
        
        // Offscreen canvas for pre-rendering next slide
        offscreenCanvas = document.createElement('canvas');
        offscreenCanvas.width = window.innerWidth;
        offscreenCanvas.height = window.innerHeight;
        offscreenCtx = offscreenCanvas.getContext('2d');
        
        // Canvas to store current frame for blending
        currentCanvas = document.createElement('canvas');
        currentCanvas.width = window.innerWidth;
        currentCanvas.height = window.innerHeight;
        currentCtx = currentCanvas.getContext('2d');
        
        window.addEventListener('resize', handleResize);
        document.addEventListener('keydown', handleKeydown);
        container.addEventListener('click', goHome);
        
        // Render first photo and start auto-advance
        renderSlide(currentIndex, ctx);
        preloadAhead();
        startDisplayTimer();
    }
    
    function handleResize() {
        const width = window.innerWidth;
        const height = window.innerHeight;
        
        canvas.width = width;
        canvas.height = height;
        offscreenCanvas.width = width;
        offscreenCanvas.height = height;
        currentCanvas.width = width;
        currentCanvas.height = height;
        
        if (isTransitioning) {
            // Re-render both frames at new size
            renderSlide(currentIndex, currentCtx);
            // The transition will continue and re-render
        } else {
            renderSlide(currentIndex, ctx);
        }
    }
    
    function renderSlide(index, targetCtx) {
        const photo = photos[index];
        const img = preloadedImages.get(photo.url);
        const caption = photo.caption || '';
        
        if (img && img.complete) {
            renderPhotoWithConfig(img, photo.config, caption, targetCtx, targetCtx.canvas);
        } else {
            // Image not loaded yet, load it
            const newImg = new Image();
            newImg.onload = function() {
                preloadedImages.set(photo.url, newImg);
                renderPhotoWithConfig(newImg, photo.config, caption, targetCtx, targetCtx.canvas);
            };
            newImg.src = photo.url;
        }
    }
    
    function renderPhotoWithConfig(img, config, caption, targetCtx, targetCanvas) {
        targetCtx.clearRect(0, 0, targetCanvas.width, targetCanvas.height);
        
        const crop = config.crop || 'Letterbox';
        const background = config.background || 'Black';
        
        drawBackground(img, background, targetCtx, targetCanvas);
        drawCroppedImage(img, crop, targetCtx, targetCanvas);
        drawCaption(caption, targetCtx, targetCanvas);
    }
    
    function drawCaption(caption, targetCtx, targetCanvas) {
        if (!caption) return;
        
        const fontSize = Math.round(targetCanvas.width * 0.025); // 2.5% of canvas width
        const padding = Math.round(targetCanvas.width * 0.02);
        const yPos = targetCanvas.height - (fontSize * 1.5);
        
        targetCtx.save();
        targetCtx.font = `${fontSize}px sans-serif`;
        targetCtx.fillStyle = '#fff';
        targetCtx.shadowColor = 'rgba(0, 0, 0, 0.9)';
        targetCtx.shadowBlur = 8;
        targetCtx.shadowOffsetX = 3;
        targetCtx.shadowOffsetY = 3;
        targetCtx.textBaseline = 'bottom';
        targetCtx.fillText(caption, padding, yPos);
        targetCtx.restore();
    }
    
    function drawBackground(img, background, targetCtx, targetCanvas) {
        if (typeof background === 'string') {
            if (background === 'Black') {
                targetCtx.fillStyle = '#000';
                targetCtx.fillRect(0, 0, targetCanvas.width, targetCanvas.height);
            }
        } else if (background.Color) {
            const { r, g, b } = background.Color;
            targetCtx.fillStyle = `rgb(${Math.round(r)},${Math.round(g)},${Math.round(b)})`;
            targetCtx.fillRect(0, 0, targetCanvas.width, targetCanvas.height);
        } else if (background.Gaussian) {
            const blurR = background.Gaussian.r || 10;
            targetCtx.save();
            targetCtx.filter = `blur(${blurR}px)`;
            drawFillImage(img, targetCtx, targetCanvas);
            targetCtx.restore();
        }
    }
    
    function drawFillImage(img, targetCtx, targetCanvas) {
        const canvasRatio = targetCanvas.width / targetCanvas.height;
        const imgRatio = img.width / img.height;
        
        let drawWidth, drawHeight, drawX, drawY;
        
        if (imgRatio > canvasRatio) {
            drawHeight = targetCanvas.height;
            drawWidth = drawHeight * imgRatio;
            drawX = (targetCanvas.width - drawWidth) / 2;
            drawY = 0;
        } else {
            drawWidth = targetCanvas.width;
            drawHeight = drawWidth / imgRatio;
            drawX = 0;
            drawY = (targetCanvas.height - drawHeight) / 2;
        }
        
        targetCtx.drawImage(img, drawX, drawY, drawWidth, drawHeight);
    }
    
    function drawCroppedImage(img, crop, targetCtx, targetCanvas) {
        const canvasRatio = targetCanvas.width / targetCanvas.height;
        const imgRatio = img.width / img.height;
        
        let drawWidth, drawHeight, drawX, drawY;
        
        if (typeof crop === 'string' && crop === 'Letterbox') {
            if (imgRatio > canvasRatio) {
                drawWidth = targetCanvas.width;
                drawHeight = drawWidth / imgRatio;
                drawX = 0;
                drawY = (targetCanvas.height - drawHeight) / 2;
            } else {
                drawHeight = targetCanvas.height;
                drawWidth = drawHeight * imgRatio;
                drawX = (targetCanvas.width - drawWidth) / 2;
                drawY = 0;
            }
            targetCtx.drawImage(img, drawX, drawY, drawWidth, drawHeight);
        } else if (crop.Expand) {
            const dx = crop.Expand.dx || 0;
            const dy = crop.Expand.dy || 0;
            
            if (imgRatio > canvasRatio) {
                drawHeight = targetCanvas.height;
                drawWidth = drawHeight * imgRatio;
            } else {
                drawWidth = targetCanvas.width;
                drawHeight = drawWidth / imgRatio;
            }
            
            const overflowX = drawWidth - targetCanvas.width;
            const overflowY = drawHeight - targetCanvas.height;
            
            drawX = (targetCanvas.width - drawWidth) / 2 + dx * (overflowX / 2);
            drawY = (targetCanvas.height - drawHeight) / 2 + dy * (overflowY / 2);
            
            targetCtx.drawImage(img, drawX, drawY, drawWidth, drawHeight);
        } else if (crop.Zoom) {
            const z = crop.Zoom.z || 1;
            const dx = crop.Zoom.dx || 0;
            const dy = crop.Zoom.dy || 0;
            
            if (imgRatio > canvasRatio) {
                drawWidth = targetCanvas.width;
                drawHeight = drawWidth / imgRatio;
            } else {
                drawHeight = targetCanvas.height;
                drawWidth = drawHeight * imgRatio;
            }
            
            drawWidth *= z;
            drawHeight *= z;
            
            const overflowX = Math.max(0, drawWidth - targetCanvas.width);
            const overflowY = Math.max(0, drawHeight - targetCanvas.height);
            
            drawX = (targetCanvas.width - drawWidth) / 2 + dx * (overflowX / 2);
            drawY = (targetCanvas.height - drawHeight) / 2 + dy * (overflowY / 2);
            
            targetCtx.drawImage(img, drawX, drawY, drawWidth, drawHeight);
        } else {
            // Fallback to letterbox
            if (imgRatio > canvasRatio) {
                drawWidth = targetCanvas.width;
                drawHeight = drawWidth / imgRatio;
                drawX = 0;
                drawY = (targetCanvas.height - drawHeight) / 2;
            } else {
                drawHeight = targetCanvas.height;
                drawWidth = drawHeight * imgRatio;
                drawX = (targetCanvas.width - drawWidth) / 2;
                drawY = 0;
            }
            targetCtx.drawImage(img, drawX, drawY, drawWidth, drawHeight);
        }
    }
    
    function preloadAhead() {
        for (let i = 1; i <= PRELOAD_COUNT; i++) {
            const nextIndex = (currentIndex + i) % photos.length;
            const photo = photos[nextIndex];
            
            if (!preloadedImages.has(photo.url)) {
                const img = new Image();
                img.src = photo.url;
                preloadedImages.set(photo.url, img);
            }
        }
    }
    
    // Timer management
    function cancelAllTimers() {
        if (displayTimerId !== null) {
            clearTimeout(displayTimerId);
            displayTimerId = null;
        }
        if (transitionAnimationId !== null) {
            cancelAnimationFrame(transitionAnimationId);
            transitionAnimationId = null;
        }
        isTransitioning = false;
    }
    
    function startDisplayTimer() {
        cancelAllTimers();
        
        displayTimerId = setTimeout(function() {
            displayTimerId = null;
            const nextIndex = (currentIndex + 1) % photos.length;
            startTransition(nextIndex, TRANSITION_DURATION);
        }, DISPLAY_DURATION);
    }
    
    function startTransition(targetIndex, duration) {
        cancelAllTimers();
        isTransitioning = true;
        transitionTargetIndex = targetIndex;
        
        // Save current frame to currentCanvas for blending
        currentCtx.drawImage(canvas, 0, 0);
        
        // Pre-render target slide to offscreen canvas
        renderSlide(targetIndex, offscreenCtx);
        preloadAhead();
        
        const startTime = performance.now();
        
        function animate(currentTime) {
            const elapsed = currentTime - startTime;
            const progress = Math.min(elapsed / duration, 1);
            
            // Blend current and next frames
            blendFn(ctx, currentCanvas, offscreenCanvas, progress);
            
            if (progress < 1) {
                transitionAnimationId = requestAnimationFrame(animate);
            } else {
                // Transition complete
                transitionAnimationId = null;
                isTransitioning = false;
                transitionTargetIndex = null;
                currentIndex = targetIndex;
                
                // Ensure final frame is fully rendered
                ctx.drawImage(offscreenCanvas, 0, 0);
                
                // Start next display timer
                startDisplayTimer();
            }
        }
        
        transitionAnimationId = requestAnimationFrame(animate);
    }
    
    function finishTransitionInstantly() {
        if (!isTransitioning || transitionTargetIndex === null) return;
        
        // Cancel the animation
        if (transitionAnimationId !== null) {
            cancelAnimationFrame(transitionAnimationId);
            transitionAnimationId = null;
        }
        
        // Jump to final state
        currentIndex = transitionTargetIndex;
        ctx.drawImage(offscreenCanvas, 0, 0);
        
        isTransitioning = false;
        transitionTargetIndex = null;
    }
    
    function handleKeydown(e) {
        if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
            e.preventDefault();
            // If mid-transition, just finish it instantly and restart display timer
            if (isTransitioning) {
                finishTransitionInstantly();
                startDisplayTimer();
            } else {
                // Not transitioning, start a new transition
                cancelAllTimers();
                const targetIndex = e.key === 'ArrowRight'
                    ? (currentIndex + 1) % photos.length
                    : (currentIndex - 1 + photos.length) % photos.length;
                startTransition(targetIndex, MANUAL_TRANSITION_DURATION);
            }
        } else if (e.key === 'Escape') {
            e.preventDefault();
            goHome();
        }
    }
    
    function goHome() {
        cancelAllTimers();
        window.location.href = '/';
    }
    
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
