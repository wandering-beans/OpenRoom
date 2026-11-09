export function WarningBanner({ id }: { id: string }) {
  return <div id={id} className="warning-banner" hidden>
    <span role="alert" />
    <button type="button" aria-label="警告を閉じる" title="警告を閉じる">×</button>
  </div>;
}

export function bindWarningBanner(banner: HTMLElement) {
  const message = banner.querySelector('span')!;
  const close = banner.querySelector('button')!;
  let timer: number | undefined;
  const pause = () => window.clearTimeout(timer);

  function dismiss() {
    pause();
    banner.hidden = true;
    message.textContent = '';
  }
  function resume() {
    pause();
    if (!banner.hidden && !banner.matches(':hover, :focus-within')) {
      timer = window.setTimeout(dismiss, 8000);
    }
  }

  close.onclick = dismiss;
  close.onfocus = pause;
  close.onblur = resume;
  banner.onmouseenter = pause;
  banner.onmouseleave = resume;
  return {
    dismiss,
    show(error: unknown) {
      banner.hidden = false;
      message.textContent = String(error);
      resume();
    },
  };
}
