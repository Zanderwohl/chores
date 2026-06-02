(function() {
    let sleepTimerId = null;
    
    function getSettingsFromCookie() {
        const cookies = document.cookie.split(';');
        for (const cookie of cookies) {
            const [name, value] = cookie.trim().split('=');
            if (name === 'settings' && value) {
                try {
                    return JSON.parse(decodeURIComponent(value));
                } catch (e) {
                    return null;
                }
            }
        }
        return null;
    }
    
    function getSleepTimeMs() {
        const settings = getSettingsFromCookie();
        if (settings && settings.sleep_time) {
            return settings.sleep_time * 60 * 1000;
        }
        return null;
    }
    
    function goToSleep() {
        window.location.href = '/idle';
    }
    
    function resetSleepTimer() {
        if (sleepTimerId !== null) {
            clearTimeout(sleepTimerId);
            sleepTimerId = null;
        }
        
        const sleepTimeMs = getSleepTimeMs();
        if (sleepTimeMs !== null) {
            sleepTimerId = setTimeout(goToSleep, sleepTimeMs);
        }
    }
    
    function init() {
        // Don't run on the idle page itself
        if (window.location.pathname === '/idle') {
            return;
        }
        
        // Start the sleep timer
        resetSleepTimer();
        
        // Reset timer on any user interaction (capture phase, doesn't block propagation)
        document.addEventListener('click', resetSleepTimer, true);
        document.addEventListener('touchstart', resetSleepTimer, true);
        
        // Reset timer before any HTMX request
        document.body.addEventListener('htmx:beforeRequest', resetSleepTimer);
    }
    
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
