export function recordDownloadClick(platform) {
  if (!['windows', 'macos'].includes(platform)) return;
  // Keep the request alive during navigation; counting must never block a download.
  void fetch(`/api/download-click/${platform}`, {
    method: 'POST', keepalive: true, credentials: 'omit', cache: 'no-store'
  }).catch(() => {});
}

export function setupDownloadForm(form, navigate = url => window.location.assign(url), recordClick = recordDownloadClick) {
  const agreement = form.querySelector('#terms-agreement');
  const buttons = [...form.querySelectorAll('[data-platform]')];
  const destinations = new Map();
  // Consent is specific to this visit; never restore it from storage or browser form state.
  agreement.checked = false;
  const update = () => {
    for (const button of buttons) button.disabled = !agreement.checked || !destinations.has(button.value);
  };
  agreement.addEventListener('change', update);
  form.addEventListener('submit', event => {
    event.preventDefault();
    if (!agreement.checked || !form.reportValidity()) return;
    const destination = destinations.get(event.submitter?.value);
    if (!destination) return;
    if (destination.countClick) {
      try { recordClick(event.submitter.value); } catch { /* Downloads still work if tracking is unavailable. */ }
    }
    navigate(destination.url);
  });
  window.addEventListener('pageshow', event => {
    if (event.persisted) {agreement.checked = false; update();}
  });
  update();
  return {setDestination(platform, url, countClick = false) {destinations.set(platform, {url, countClick}); update();}};
}
