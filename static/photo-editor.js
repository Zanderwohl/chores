(function() {
    const canvas = document.getElementById('preview-canvas');
    if (!canvas) return;
    
    const ctx = canvas.getContext('2d');
    const img = new Image();
    let imageLoaded = false;
    
    img.onload = function() {
        imageLoaded = true;
        updateCropControls(); // Re-run with actual image dimensions
        updatePreview();
    };
    img.src = window.PHOTO_URL;
    
    window.updatePreview = function() {
        if (!imageLoaded) return;
        
        const cropType = getRadioValue('crop_type');
        const bgType = getRadioValue('bg_type');
        
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        
        drawBackground(bgType);
        drawCroppedImage(cropType);
        drawCaption();
        
        updateSliderLabels();
    };
    
    window.updateCropControls = function() {
        const cropType = getRadioValue('crop_type');
        
        const zoomSlider = document.getElementById('crop_z');
        const dxSlider = document.getElementById('crop_dx');
        const dySlider = document.getElementById('crop_dy');
        
        const zoomGroup = document.getElementById('crop_z_group');
        const dxGroup = document.getElementById('crop_dx_group');
        const dyGroup = document.getElementById('crop_dy_group');
        
        if (!zoomSlider || !dxSlider || !dySlider) return;
        
        // Calculate aspect ratios for expand mode logic
        const canvasRatio = canvas.width / canvas.height;
        const imgRatio = imageLoaded ? (img.width / img.height) : canvasRatio;
        const hasHorizontalOverflow = imgRatio > canvasRatio;
        const hasVerticalOverflow = imgRatio < canvasRatio;
        
        if (cropType === 'letterbox') {
            zoomSlider.disabled = true;
            dxSlider.disabled = true;
            dySlider.disabled = true;
            zoomGroup.classList.add('disabled');
            dxGroup.classList.add('disabled');
            dyGroup.classList.add('disabled');
        } else if (cropType === 'expand') {
            zoomSlider.disabled = true;
            zoomGroup.classList.add('disabled');
            
            // Only enable the slider for the direction that has overflow
            if (hasHorizontalOverflow) {
                dxSlider.disabled = false;
                dySlider.disabled = true;
                dxGroup.classList.remove('disabled');
                dyGroup.classList.add('disabled');
            } else if (hasVerticalOverflow) {
                dxSlider.disabled = true;
                dySlider.disabled = false;
                dxGroup.classList.add('disabled');
                dyGroup.classList.remove('disabled');
            } else {
                // Perfect match - no overflow either way
                dxSlider.disabled = true;
                dySlider.disabled = true;
                dxGroup.classList.add('disabled');
                dyGroup.classList.add('disabled');
            }
        } else if (cropType === 'zoom') {
            // Set minimum zoom to 1.0 (letterbox) - can't zoom out beyond touching 2 edges
            zoomSlider.min = '1';
            if (parseFloat(zoomSlider.value) < 1) {
                zoomSlider.value = '1';
            }
            
            zoomSlider.disabled = false;
            zoomGroup.classList.remove('disabled');
            
            // In zoom mode, enable offsets only if there's overflow at current zoom level
            const z = parseFloat(zoomSlider.value) || 1;
            let baseWidth, baseHeight;
            if (imgRatio > canvasRatio) {
                baseWidth = canvas.width;
                baseHeight = baseWidth / imgRatio;
            } else {
                baseHeight = canvas.height;
                baseWidth = baseHeight * imgRatio;
            }
            const zoomedWidth = baseWidth * z;
            const zoomedHeight = baseHeight * z;
            const zoomHasHorizontalOverflow = zoomedWidth > canvas.width;
            const zoomHasVerticalOverflow = zoomedHeight > canvas.height;
            
            if (zoomHasHorizontalOverflow) {
                dxSlider.disabled = false;
                dxGroup.classList.remove('disabled');
            } else {
                dxSlider.disabled = true;
                dxGroup.classList.add('disabled');
            }
            
            if (zoomHasVerticalOverflow) {
                dySlider.disabled = false;
                dyGroup.classList.remove('disabled');
            } else {
                dySlider.disabled = true;
                dyGroup.classList.add('disabled');
            }
        }
    };
    
    window.updateBgControls = function() {
        const bgType = getRadioValue('bg_type');
        
        const rSlider = document.getElementById('bg_r');
        const gSlider = document.getElementById('bg_g');
        const bSlider = document.getElementById('bg_b');
        const blurSlider = document.getElementById('bg_blur_r');
        
        const rGroup = document.getElementById('bg_r_group');
        const gGroup = document.getElementById('bg_g_group');
        const bGroup = document.getElementById('bg_b_group');
        const blurGroup = document.getElementById('bg_blur_r_group');
        
        if (bgType === 'black') {
            rSlider.disabled = true;
            gSlider.disabled = true;
            bSlider.disabled = true;
            blurSlider.disabled = true;
            rGroup.classList.add('disabled');
            gGroup.classList.add('disabled');
            bGroup.classList.add('disabled');
            blurGroup.classList.add('disabled');
        } else if (bgType === 'color') {
            rSlider.disabled = false;
            gSlider.disabled = false;
            bSlider.disabled = false;
            blurSlider.disabled = true;
            rGroup.classList.remove('disabled');
            gGroup.classList.remove('disabled');
            bGroup.classList.remove('disabled');
            blurGroup.classList.add('disabled');
        } else if (bgType === 'gaussian') {
            rSlider.disabled = true;
            gSlider.disabled = true;
            bSlider.disabled = true;
            blurSlider.disabled = false;
            rGroup.classList.add('disabled');
            gGroup.classList.add('disabled');
            bGroup.classList.add('disabled');
            blurGroup.classList.remove('disabled');
        }
    };
    
    function getRadioValue(name) {
        const el = document.querySelector(`input[name="${name}"]:checked`);
        return el ? el.value : '';
    }
    
    function getValue(id) {
        const el = document.getElementById(id);
        return el ? parseFloat(el.value) || 0 : 0;
    }
    
    function updateSliderLabels() {
        const labels = [
            ['crop_dx', 'crop_dx_val'],
            ['crop_dy', 'crop_dy_val'],
            ['crop_z', 'crop_z_val'],
            ['bg_r', 'bg_r_val'],
            ['bg_g', 'bg_g_val'],
            ['bg_b', 'bg_b_val'],
            ['bg_blur_r', 'bg_blur_r_val']
        ];
        
        for (const [inputId, labelId] of labels) {
            const input = document.getElementById(inputId);
            const label = document.getElementById(labelId);
            if (input && label) {
                const val = parseFloat(input.value);
                if (inputId.startsWith('crop_z')) {
                    label.textContent = val.toFixed(2);
                } else if (inputId.startsWith('crop_')) {
                    label.textContent = val.toFixed(2);
                } else {
                    label.textContent = Math.round(val);
                }
            }
        }
    }
    
    function drawBackground(bgType) {
        if (bgType === 'black') {
            ctx.fillStyle = '#000';
            ctx.fillRect(0, 0, canvas.width, canvas.height);
        } else if (bgType === 'color') {
            const r = Math.round(getValue('bg_r'));
            const g = Math.round(getValue('bg_g'));
            const b = Math.round(getValue('bg_b'));
            ctx.fillStyle = `rgb(${r},${g},${b})`;
            ctx.fillRect(0, 0, canvas.width, canvas.height);
        } else if (bgType === 'gaussian') {
            const blurR = getValue('bg_blur_r');
            ctx.save();
            ctx.filter = `blur(${blurR}px)`;
            drawFillImage();
            ctx.restore();
        }
    }
    
    function drawFillImage() {
        const canvasRatio = canvas.width / canvas.height;
        const imgRatio = img.width / img.height;
        
        let drawWidth, drawHeight, drawX, drawY;
        
        if (imgRatio > canvasRatio) {
            drawHeight = canvas.height;
            drawWidth = drawHeight * imgRatio;
            drawX = (canvas.width - drawWidth) / 2;
            drawY = 0;
        } else {
            drawWidth = canvas.width;
            drawHeight = drawWidth / imgRatio;
            drawX = 0;
            drawY = (canvas.height - drawHeight) / 2;
        }
        
        ctx.drawImage(img, drawX, drawY, drawWidth, drawHeight);
    }
    
    function drawCroppedImage(cropType) {
        const canvasRatio = canvas.width / canvas.height;
        const imgRatio = img.width / img.height;
        
        let drawWidth, drawHeight, drawX, drawY;
        
        if (cropType === 'letterbox') {
            if (imgRatio > canvasRatio) {
                drawWidth = canvas.width;
                drawHeight = drawWidth / imgRatio;
                drawX = 0;
                drawY = (canvas.height - drawHeight) / 2;
            } else {
                drawHeight = canvas.height;
                drawWidth = drawHeight * imgRatio;
                drawX = (canvas.width - drawWidth) / 2;
                drawY = 0;
            }
            ctx.drawImage(img, drawX, drawY, drawWidth, drawHeight);
        } else if (cropType === 'expand') {
            const dx = getValue('crop_dx');
            const dy = getValue('crop_dy');
            
            if (imgRatio > canvasRatio) {
                drawHeight = canvas.height;
                drawWidth = drawHeight * imgRatio;
            } else {
                drawWidth = canvas.width;
                drawHeight = drawWidth / imgRatio;
            }
            
            const overflowX = drawWidth - canvas.width;
            const overflowY = drawHeight - canvas.height;
            
            drawX = (canvas.width - drawWidth) / 2 + dx * (overflowX / 2);
            drawY = (canvas.height - drawHeight) / 2 + dy * (overflowY / 2);
            
            ctx.drawImage(img, drawX, drawY, drawWidth, drawHeight);
        } else if (cropType === 'zoom') {
            const z = getValue('crop_z');
            const dx = getValue('crop_dx');
            const dy = getValue('crop_dy');
            
            if (imgRatio > canvasRatio) {
                drawWidth = canvas.width;
                drawHeight = drawWidth / imgRatio;
            } else {
                drawHeight = canvas.height;
                drawWidth = drawHeight * imgRatio;
            }
            
            drawWidth *= z;
            drawHeight *= z;
            
            const overflowX = Math.max(0, drawWidth - canvas.width);
            const overflowY = Math.max(0, drawHeight - canvas.height);
            
            drawX = (canvas.width - drawWidth) / 2 + dx * (overflowX / 2);
            drawY = (canvas.height - drawHeight) / 2 + dy * (overflowY / 2);
            
            ctx.drawImage(img, drawX, drawY, drawWidth, drawHeight);
        }
    }
    
    function drawCaption() {
        const captionInput = document.getElementById('caption');
        const caption = captionInput ? captionInput.value : '';
        
        if (!caption) return;
        
        const fontSize = Math.round(canvas.width * 0.025); // 2.5% of canvas width
        const padding = Math.round(canvas.width * 0.02);
        const yPos = canvas.height - (fontSize * 1.5);
        
        ctx.save();
        ctx.font = `${fontSize}px sans-serif`;
        ctx.fillStyle = '#fff';
        ctx.shadowColor = 'rgba(0, 0, 0, 0.9)';
        ctx.shadowBlur = 8;
        ctx.shadowOffsetX = 3;
        ctx.shadowOffsetY = 3;
        ctx.textBaseline = 'bottom';
        ctx.fillText(caption, padding, yPos);
        ctx.restore();
    }
    
    function initControls() {
        updateCropControls();
        updateBgControls();
    }
    
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', function() {
            initControls();
            if (imageLoaded) updatePreview();
        });
    } else {
        initControls();
    }
})();
