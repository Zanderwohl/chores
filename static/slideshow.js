(function() {
    const photos = window.SLIDESHOW_PHOTOS || [];
    const PRELOAD_COUNT = 3;
    
    let currentIndex = 0;
    let imgElement = null;
    const preloadedImages = new Map();
    
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
        
        imgElement = document.createElement('img');
        imgElement.style.cssText = `
            max-width: 100%;
            max-height: 100%;
            object-fit: contain;
        `;
        container.appendChild(imgElement);
        
        showPhoto(0);
        preloadAhead();
        
        document.addEventListener('keydown', handleKeydown);
        container.addEventListener('click', goHome);
    }
    
    function showPhoto(index) {
        if (photos.length === 0) return;
        
        currentIndex = ((index % photos.length) + photos.length) % photos.length;
        const photo = photos[currentIndex];
        
        if (preloadedImages.has(photo.url)) {
            imgElement.src = preloadedImages.get(photo.url).src;
        } else {
            imgElement.src = photo.url;
        }
        
        preloadAhead();
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
    
    function handleKeydown(e) {
        switch (e.key) {
            case 'ArrowRight':
                e.preventDefault();
                showPhoto(currentIndex + 1);
                break;
            case 'ArrowLeft':
                e.preventDefault();
                showPhoto(currentIndex - 1);
                break;
            case 'Escape':
                e.preventDefault();
                goHome();
                break;
        }
    }
    
    function goHome() {
        window.location.href = '/';
    }
    
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
