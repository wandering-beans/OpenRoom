export function StartupScreen() {
  return <>
  <section id="startup-screen" className="startup-screen" role="status" aria-label="OpenRoomを起動しています">
    <div className="startup-identity">
      <img src="branding/openroom-icon.png" width="512" height="512" alt="" />
      <h1>OpenRoom</h1>
      <p>RAW PHOTO EDITOR</p>
    </div>
    <div id="startup-loading" className="startup-status"><div className="startup-track" aria-hidden="true"></div><span>カタログを読み込んでいます…</span></div>
    <div id="catalog-setup" className="catalog-setup" hidden>
      <h2>カタログを作成</h2>
      <p>カタログ名と保存先を指定してください。</p>
      <button id="catalog-setup-new">新規作成</button>
      <button id="catalog-setup-open">既存のカタログを開く</button>
      <p id="catalog-setup-error" role="alert"></p>
    </div>
  </section>
  </>;
}
