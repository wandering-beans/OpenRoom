export function setupDownloadForm(form, navigate = url => window.location.assign(url)) {
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
    if (destination) navigate(destination);
  });
  window.addEventListener('pageshow', event => {
    if (event.persisted) {agreement.checked = false; update();}
  });
  update();
  return {setDestination(platform, url) {destinations.set(platform, url); update();}};
}
