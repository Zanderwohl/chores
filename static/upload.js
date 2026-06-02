(function() {
    'use strict';

    const ALLOWED_TYPES = ['image/jpeg', 'image/png', 'image/gif', 'image/webp'];

    const dropzone = document.getElementById('upload-dropzone');
    const fileInput = document.getElementById('file-input');
    const selectBtn = document.getElementById('select-btn');
    const preview = document.getElementById('upload-preview');
    const previewImage = document.getElementById('preview-image');
    const filenameDisplay = document.getElementById('filename-display');
    const warningDiv = document.getElementById('upload-warning');
    const clearBtn = document.getElementById('clear-btn');
    const uploadBtn = document.getElementById('upload-btn');
    const statusDiv = document.getElementById('upload-status');

    let selectedFile = null;

    selectBtn.addEventListener('click', function() {
        fileInput.click();
    });

    fileInput.addEventListener('change', function(e) {
        if (e.target.files && e.target.files[0]) {
            handleFile(e.target.files[0]);
        }
    });

    dropzone.addEventListener('dragover', function(e) {
        e.preventDefault();
        e.stopPropagation();
        dropzone.classList.add('dragover');
    });

    dropzone.addEventListener('dragleave', function(e) {
        e.preventDefault();
        e.stopPropagation();
        dropzone.classList.remove('dragover');
    });

    dropzone.addEventListener('drop', function(e) {
        e.preventDefault();
        e.stopPropagation();
        dropzone.classList.remove('dragover');

        if (e.dataTransfer.files && e.dataTransfer.files[0]) {
            handleFile(e.dataTransfer.files[0]);
        }
    });

    clearBtn.addEventListener('click', function() {
        clearSelection();
    });

    uploadBtn.addEventListener('click', function() {
        uploadFile();
    });

    function handleFile(file) {
        if (!ALLOWED_TYPES.includes(file.type)) {
            showStatus('Invalid file type. Allowed: JPEG, PNG, GIF, WebP', true);
            return;
        }

        selectedFile = file;

        const reader = new FileReader();
        reader.onload = function(e) {
            previewImage.src = e.target.result;
            filenameDisplay.textContent = file.name;
            dropzone.style.display = 'none';
            preview.style.display = 'block';
            statusDiv.innerHTML = '';
            
            checkFilename(file.name);
        };
        reader.readAsDataURL(file);
    }

    function checkFilename(filename) {
        fetch('/photos/upload/check?filename=' + encodeURIComponent(filename))
            .then(function(response) { return response.text(); })
            .then(function(html) {
                warningDiv.innerHTML = html;
            })
            .catch(function(err) {
                console.error('Error checking filename:', err);
            });
    }

    function clearSelection() {
        selectedFile = null;
        fileInput.value = '';
        previewImage.src = '';
        filenameDisplay.textContent = '';
        warningDiv.innerHTML = '';
        statusDiv.innerHTML = '';
        preview.style.display = 'none';
        dropzone.style.display = 'block';
    }

    function uploadFile() {
        if (!selectedFile) {
            showStatus('No file selected', true);
            return;
        }

        uploadBtn.disabled = true;
        clearBtn.disabled = true;
        showStatus('Uploading...', false);

        const formData = new FormData();
        formData.append('file', selectedFile);

        fetch('/photos/upload', {
            method: 'POST',
            body: formData
        })
        .then(function(response) {
            const redirect = response.headers.get('HX-Redirect');
            if (redirect) {
                window.location.href = redirect;
                return;
            }
            return response.text().then(function(html) {
                if (response.ok) {
                    showStatus('Upload successful! Redirecting...', false);
                    setTimeout(function() {
                        window.location.href = '/photos';
                    }, 1000);
                } else {
                    showStatus(html || 'Upload failed', true);
                    uploadBtn.disabled = false;
                    clearBtn.disabled = false;
                }
            });
        })
        .catch(function(err) {
            console.error('Upload error:', err);
            showStatus('Upload failed: ' + err.message, true);
            uploadBtn.disabled = false;
            clearBtn.disabled = false;
        });
    }

    function showStatus(message, isError) {
        statusDiv.innerHTML = '<p class="' + (isError ? 'upload-error' : 'upload-status-message') + '">' + message + '</p>';
    }
})();
