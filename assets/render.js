const { chromium } = require('playwright');
const fs = require('fs');
(async () => {
  const dir = process.argv[2];
  const logo = fs.readFileSync(dir + '/logo.svg', 'utf8');
  const b = await chromium.launch();
  // PNG marks
  for (const s of [512, 180]) {
    const p = await b.newPage({ viewport: { width: s, height: s } });
    await p.setContent(`<body style="margin:0;background:transparent">${logo.replace('width="128" height="128"', `width="${s}" height="${s}"`)}</body>`);
    await p.screenshot({ path: `${dir}/logo-${s}.png`, omitBackground: true });
    await p.close();
  }
  // social banner
  const p = await b.newPage({ viewport: { width: 1280, height: 640 } });
  await p.setContent(`<!doctype html><html><head><style>
  body{margin:0;width:1280px;height:640px;overflow:hidden;font-family:Inter,'Segoe UI',system-ui,sans-serif;
   background:radial-gradient(900px 500px at 85% 20%,#4F7BFF55,transparent 60%),radial-gradient(700px 500px at 10% 100%,#14C8B044,transparent 60%),radial-gradient(600px 400px at 30% 0%,#7C5CFF55,transparent 60%),#0c0a1d;color:#F4F2FF}
  .l{position:absolute;left:88px;top:150px}
  h1{font-size:96px;margin:28px 0 0;letter-spacing:-4px;font-weight:800}
  h1 span{background:linear-gradient(90deg,#9D86FF,#4F7BFF 50%,#14C8B0);-webkit-background-clip:text;color:transparent}
  p{font-size:30px;color:#B9B5DA;margin:14px 0 0;max-width:640px;line-height:1.3}
  .w{position:absolute;border-radius:24px;background:linear-gradient(180deg,#ffffff22,#ffffff0d);border:1.5px solid #ffffff33;box-shadow:0 20px 50px #0006;backdrop-filter:blur(8px);padding:22px 26px}
  .k{font-size:14px;letter-spacing:2px;color:#B9B5DA;text-transform:uppercase}
  .big{font-size:64px;font-weight:700;letter-spacing:-2px}
  .bar{height:10px;border-radius:5px;background:#ffffff22;margin-top:14px}.bar i{display:block;height:100%;border-radius:5px;background:linear-gradient(90deg,#14C8B0,#4F7BFF)}
  .chip{display:inline-block;padding:6px 12px;border-radius:99px;background:#FFB54733;color:#FFD9A0;font-size:14px;margin-top:12px}
  </style></head><body>
  <div class="l">${logo.replace('width="128" height="128"','width="112" height="112"')}<h1>uni<span>widged</span></h1>
  <p>Sandboxed, GPU-rendered desktop widgets<br>that cost nothing when nothing happens.</p></div>
  <div class="w" style="left:830px;top:90px;width:330px"><div class="k">Monday, 9 November</div><div class="big">14:32</div></div>
  <div class="w" style="left:780px;top:270px;width:240px"><div class="k">CPU</div><div style="font-size:40px;font-weight:700">23%</div><div class="bar"><i style="width:23%"></i></div>
   <svg width="240" height="54" viewBox="0 0 240 54" style="margin-top:12px"><defs><linearGradient id="a" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="#4F7BFF" stop-opacity=".5"/><stop offset="1" stop-color="#4F7BFF" stop-opacity="0"/></linearGradient></defs><path d="M0 40 L20 34 L40 38 L60 22 L80 28 L100 18 L120 30 L140 26 L160 12 L180 20 L200 16 L220 24 L236 14 V54 H0Z" fill="url(#a)"/><path d="M0 40 L20 34 L40 38 L60 22 L80 28 L100 18 L120 30 L140 26 L160 12 L180 20 L200 16 L220 24 L236 14" fill="none" stroke="#7FA0FF" stroke-width="2.5"/><circle cx="236" cy="14" r="5" fill="#FFB547"/></svg></div>
  <div class="w" style="left:1050px;top:300px;width:150px"><div class="k">Battery</div><div style="font-size:40px;font-weight:700">87%</div><div class="chip">on mains</div></div>
  <div class="w" style="left:860px;top:480px;width:300px;padding:16px 22px"><div class="k">sandboxed · wasm32-wasip2</div><div style="font-size:18px;margin-top:6px;color:#DCD8FF">net:api.open-meteo.com · fs:cache</div></div>
  </body></html>`);
  await p.screenshot({ path: `${dir}/social-preview.png` });
  await b.close();
})();
