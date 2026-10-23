// The practice layer's look: a soft amber the shell never uses, on light
// cards, so the eye finds the guide at once without it shouting over the work.

export const COACH_CSS = `
.pc-layer{--pc-acc:#e9c27a;--pc-acc2:#e3a77a;--pc-ink:#2a1f0e;--pc-card:#f7f3ea;--pc-text:#24211b;--pc-mute:#736c5f;--pc-line:#e6dcc6;--pc-ease:cubic-bezier(.2,.8,.2,1);
  position:fixed;inset:0;z-index:60;pointer-events:none;font-family:Pretendard,-apple-system,system-ui,sans-serif}
.pc-spot{position:absolute;border-radius:12px;box-shadow:0 0 0 2px var(--pc-acc),0 0 0 7px color-mix(in srgb,var(--pc-acc) 16%,transparent),0 0 0 4000px rgba(6,6,10,.42);
  transition:left .6s var(--pc-ease),top .6s var(--pc-ease),width .6s var(--pc-ease),height .6s var(--pc-ease),box-shadow .4s}
.pc-spot::after{content:"";position:absolute;inset:-2px;border-radius:14px;border:1.5px solid var(--pc-acc);animation:pc-pulse 2.6s ease-out infinite}
.pc-spot.off{box-shadow:0 0 0 0 transparent,0 0 0 0 transparent,0 0 0 4000px rgba(6,6,10,0)}
.pc-spot.off::after{display:none}
@keyframes pc-pulse{0%{opacity:.55;transform:scale(1)}100%{opacity:0;transform:scale(1.03)}}
.pc-zdim{position:absolute;inset:0;background:rgba(6,6,10,.32);animation:pc-in .4s both}
.pc-zone{position:absolute;border-radius:12px;border:2px dashed;animation:pc-zone .5s var(--pc-ease) both}
.pc-zone.z1{border-color:#a9cfe2;background:rgba(169,207,226,.06)}.pc-zone.z1 .pc-tag{background:#cfe3ee;color:#0e1a24}
.pc-zone.z2{border-color:#c3bbe3;background:rgba(195,187,227,.06)}.pc-zone.z2 .pc-tag{background:#ddd8f0;color:#17122a}
.pc-zone.z3{border-color:#e6cf98;background:rgba(230,207,152,.06)}.pc-zone.z3 .pc-tag{background:#efe2c2;color:#2a1f0e}
.pc-tag{position:absolute;left:50%;top:42%;transform:translate(-50%,-50%);padding:10px 16px;border-radius:14px;font-weight:700;font-size:18px;white-space:nowrap;box-shadow:0 10px 30px rgba(0,0,0,.45);text-align:center;line-height:1.35}
.pc-tag small{display:block;font-size:12.5px;font-weight:500;opacity:.8}
@keyframes pc-zone{from{opacity:0;transform:scale(.98)}to{opacity:1;transform:scale(1)}}
@keyframes pc-in{from{opacity:0}to{opacity:1}}
.pc-bubble{position:absolute;width:340px;background:var(--pc-card);color:var(--pc-text);border-radius:16px;padding:16px 18px 14px 20px;pointer-events:auto;
  box-shadow:0 18px 50px rgba(0,0,0,.5),0 0 0 1px var(--pc-line);transition:left .6s var(--pc-ease),top .6s var(--pc-ease);animation:pc-bubble .35s var(--pc-ease) both}
@keyframes pc-bubble{from{opacity:0;transform:translateY(6px)}to{opacity:1;transform:none}}
.pc-bubble::before{content:"";position:absolute;inset:0 auto 0 0;width:4px;border-radius:16px 0 0 16px;background:var(--pc-acc)}
.pc-arrow{position:absolute;width:16px;height:16px;background:var(--pc-card);transform:rotate(45deg);box-shadow:-1px -1px 0 var(--pc-line)}
.pc-kick{font-size:11.5px;font-weight:600;letter-spacing:.02em;color:color-mix(in srgb,var(--pc-acc2) 60%,#5a4a30);display:flex;justify-content:space-between;gap:8px}
.pc-bubble h3{margin:6px 0 4px;font-size:17px;line-height:1.35;font-weight:600}
.pc-bubble p{margin:0;font-size:13.5px;line-height:1.55;color:var(--pc-mute)}
.pc-via{margin-top:8px;font-size:12px;color:var(--pc-mute);display:flex;gap:6px;align-items:center;flex-wrap:wrap}
.pc-row{display:flex;align-items:center;gap:8px;margin-top:12px}
.pc-dots{display:flex;gap:5px;margin-right:auto}
.pc-dots i{width:6px;height:6px;border-radius:9px;background:var(--pc-line);transition:all .3s}
.pc-dots i.on{width:18px;background:var(--pc-acc)}
.pc-dots i.done{background:color-mix(in srgb,var(--pc-acc) 55%,var(--pc-line))}
.pc-go{font:inherit;font-size:13px;font-weight:600;border:0;border-radius:10px;padding:8px 12px;background:var(--pc-acc);color:var(--pc-ink);cursor:pointer;display:flex;gap:8px;align-items:center}
.pc-back{font:inherit;font-size:12.5px;border:0;background:none;color:var(--pc-mute);cursor:pointer;padding:6px}
.pc-keys{display:inline-flex;gap:3px;vertical-align:middle}
.pc-kc{display:inline-flex;align-items:center;justify-content:center;min-width:22px;height:22px;padding:0 6px;border-radius:6px;font-size:12px;font-weight:700;background:#fff;color:#1d1a14;box-shadow:0 1px 0 #d9cdb2;border:1px solid #e3d6b8}
.pc-go .pc-kc{background:rgba(255,255,255,.55);border-color:transparent;box-shadow:none}
.pc-flash{position:absolute;left:50%;top:40%;transform:translate(-50%,-50%);display:flex;gap:10px;animation:pc-flash .9s var(--pc-ease) both}
.pc-flash .pc-kc{min-width:56px;height:56px;font-size:26px;border-radius:12px;padding:0 14px;background:var(--pc-card);box-shadow:0 3px 0 #cfc4ac,0 16px 32px rgba(0,0,0,.4);border:1px solid var(--pc-line)}
@keyframes pc-flash{0%{opacity:0;transform:translate(-50%,-50%) scale(.85)}20%{opacity:1;transform:translate(-50%,-50%) scale(1)}75%{opacity:1}100%{opacity:0}}
.pc-mode{position:absolute;left:50%;top:5px;transform:translateX(-50%);font-size:12px;font-weight:600;padding:4px 12px;border-radius:99px;background:var(--pc-card);color:var(--pc-text);box-shadow:0 0 0 1px var(--pc-line)}
.pc-quest{position:absolute;left:16px;bottom:16px;width:280px;background:var(--pc-card);color:var(--pc-text);border-radius:16px;overflow:hidden;pointer-events:auto;box-shadow:0 18px 50px rgba(0,0,0,.55),0 0 0 1px var(--pc-line);animation:pc-quest .5s var(--pc-ease) both}
@keyframes pc-quest{from{transform:translateY(30px);opacity:0}to{transform:none;opacity:1}}
.pc-qh{padding:12px 14px 10px;background:color-mix(in srgb,var(--pc-acc) 38%,var(--pc-card));color:var(--pc-ink)}
.pc-qh small{display:block;font-size:11px;font-weight:700;opacity:.75}
.pc-qh b{font-size:15px}
.pc-bar{height:5px;border-radius:9px;background:rgba(0,0,0,.12);margin-top:8px;overflow:hidden}
.pc-bar i{display:block;height:100%;background:color-mix(in srgb,var(--pc-acc2) 80%,#000);border-radius:9px;transition:width .5s var(--pc-ease)}
.pc-quest ol{list-style:none;margin:0;padding:8px}
.pc-quest li{display:flex;align-items:center;gap:9px;padding:6px 8px;border-radius:9px;font-size:13px;color:var(--pc-mute);transition:background .3s,color .3s}
.pc-quest li.cur{background:color-mix(in srgb,var(--pc-acc) 18%,transparent);color:var(--pc-text);font-weight:600}
.pc-quest li.done{color:var(--pc-text)}
.pc-ck{width:18px;height:18px;border-radius:6px;border:1.5px solid var(--pc-line);display:grid;place-items:center;flex:none;transition:all .3s var(--pc-ease)}
.pc-quest li.done .pc-ck{background:var(--pc-acc);border-color:var(--pc-acc)}
.pc-quest li.done .pc-ck::after{content:"";width:5px;height:9px;border:solid var(--pc-ink);border-width:0 2px 2px 0;transform:rotate(45deg) translate(-1px,-1px);animation:pc-pop .35s var(--pc-ease)}
@keyframes pc-pop{0%{transform:rotate(45deg) scale(0)}100%{transform:rotate(45deg) scale(1)}}
.pc-k{margin-left:auto}
.pc-qf{padding:0 14px 12px;font-size:12px;color:var(--pc-mute);display:flex;justify-content:space-between}
.pc-qf button{font:inherit;border:0;background:none;color:var(--pc-mute);cursor:pointer;padding:0}
.pc-qf button:hover{color:var(--pc-text)}
`;
