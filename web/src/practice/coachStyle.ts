// The practice layer's look: the practice palette (`--practice-*`), a soft
// amber the shell never uses, on light cards, so the eye finds the guide at
// once without it shouting over the work. Every value comes from the tokens.

const EASE = "cubic-bezier(.2,.8,.2,1)";
const SHADOW = `0 var(--spacing-lg) var(--spacing-xxxl) color-mix(in srgb, var(--practice-scrim) 50%, transparent)`;
const ZONES = (["view", "work", "project"] as const)
  .map((zone) => `.pc-zone.${zone}{border-color:var(--practice-zone-${zone});background:color-mix(in srgb,var(--practice-zone-${zone}) 6%,transparent)}
.pc-zone.${zone} .pc-tag{background:color-mix(in srgb,var(--practice-zone-${zone}) 55%,var(--practice-keycap))}`)
  .join("\n");

export const COACH_CSS = `
.pc-layer{position:fixed;inset:0;z-index:60;pointer-events:none}
.pc-spot{position:absolute;border-radius:var(--radius-xl);box-shadow:0 0 0 var(--size-tab-indicator) var(--practice-accent),0 0 0 var(--spacing-sm) color-mix(in srgb,var(--practice-accent) 16%,transparent),0 0 0 100vmax color-mix(in srgb,var(--practice-scrim) 42%,transparent);
  transition:left .6s ${EASE},top .6s ${EASE},width .6s ${EASE},height .6s ${EASE},box-shadow .4s}
.pc-spot::after{content:"";position:absolute;inset:calc(var(--size-tab-indicator) * -1);border-radius:var(--radius-xl);border:var(--size-tab-indicator) solid var(--practice-accent);animation:pc-pulse 2.6s ease-out infinite}
.pc-spot.off{box-shadow:0 0 0 0 transparent,0 0 0 0 transparent,0 0 0 100vmax transparent}
.pc-spot.off::after{display:none}
@keyframes pc-pulse{0%{opacity:.55;transform:scale(1)}100%{opacity:0;transform:scale(1.03)}}
.pc-zdim{position:absolute;inset:0;background:color-mix(in srgb,var(--practice-scrim) 32%,transparent);animation:pc-in .4s both}
.pc-zone{position:absolute;border-radius:var(--radius-xl);border:var(--size-tab-indicator) dashed;animation:pc-zone .5s ${EASE} both}
${ZONES}
.pc-tag{position:absolute;left:50%;top:42%;transform:translate(-50%,-50%);padding:var(--spacing-md) var(--spacing-lg);border-radius:var(--radius-xl);font-weight:700;font-size:var(--text-headline);white-space:nowrap;box-shadow:${SHADOW};text-align:center;line-height:1.35;color:var(--practice-ink)}
.pc-tag small{display:block;font-size:var(--text-body);font-weight:500;opacity:.8}
@keyframes pc-zone{from{opacity:0;transform:scale(.98)}to{opacity:1;transform:scale(1)}}
@keyframes pc-in{from{opacity:0}to{opacity:1}}
.pc-bubble{position:absolute;width:var(--size-practice-bubble);background:var(--practice-card);color:var(--practice-text);border-radius:var(--radius-xl);padding:var(--spacing-lg) var(--spacing-lg) var(--spacing-md) var(--spacing-xl);pointer-events:auto;
  box-shadow:${SHADOW},0 0 0 var(--size-hairline) var(--practice-line);transition:left .6s ${EASE},top .6s ${EASE};animation:pc-bubble .35s ${EASE} both}
@keyframes pc-bubble{from{opacity:0;transform:translateY(var(--spacing-sm))}to{opacity:1;transform:none}}
.pc-bubble::before{content:"";position:absolute;inset:0 auto 0 0;width:var(--spacing-xs);border-radius:var(--radius-xl) 0 0 var(--radius-xl);background:var(--practice-accent)}
.pc-arrow{position:absolute;width:var(--spacing-lg);height:var(--spacing-lg);background:var(--practice-card);transform:rotate(45deg);box-shadow:calc(var(--size-hairline) * -1) calc(var(--size-hairline) * -1) 0 var(--practice-line)}
.pc-kick{font-size:var(--text-caption);font-weight:600;letter-spacing:.02em;color:color-mix(in srgb,var(--practice-accent-strong) 60%,var(--practice-ink));display:flex;justify-content:space-between;gap:var(--spacing-sm)}
.pc-kick span:last-child{flex:none;white-space:nowrap}
.pc-bubble h3{margin:var(--spacing-sm) 0 var(--spacing-xs);font-size:var(--text-headline);line-height:1.35;font-weight:600}
.pc-bubble p{margin:0;font-size:var(--text-subhead);line-height:1.55;color:var(--practice-mute)}
.pc-via{margin-top:var(--spacing-sm);font-size:var(--text-body);color:var(--practice-mute);display:flex;gap:var(--spacing-sm);align-items:center;flex-wrap:wrap}
.pc-row{display:flex;align-items:center;gap:var(--spacing-sm);margin-top:var(--spacing-md)}
.pc-dots{display:flex;gap:var(--spacing-xs);margin-right:auto}
.pc-dots i{width:var(--size-tab-status-dot);height:var(--size-tab-status-dot);border-radius:var(--radius-xl);background:var(--practice-line);transition:all .3s}
.pc-dots i.on{width:var(--spacing-lg);background:var(--practice-accent)}
.pc-dots i.done{background:color-mix(in srgb,var(--practice-accent) 55%,var(--practice-line))}
.pc-go{font:inherit;font-size:var(--text-subhead);font-weight:600;border:0;border-radius:var(--radius-lg);padding:var(--spacing-sm) var(--spacing-md);background:var(--practice-accent);color:var(--practice-ink);cursor:pointer;display:flex;gap:var(--spacing-sm);align-items:center}
.pc-back{font:inherit;font-size:var(--text-body);border:0;background:none;color:var(--practice-mute);cursor:pointer;padding:var(--spacing-xs)}
.pc-keys{display:inline-flex;gap:var(--spacing-xxs);vertical-align:middle}
.pc-kc{display:inline-flex;align-items:center;justify-content:center;min-width:var(--size-control-compact);height:var(--size-control-compact);padding:0 var(--spacing-xs);border-radius:var(--radius-sm);font-size:var(--text-body);font-weight:700;background:var(--practice-keycap);color:var(--practice-keycap-ink);box-shadow:0 var(--size-hairline) 0 var(--practice-line);border:var(--size-hairline) solid var(--practice-line)}
.pc-go .pc-kc{background:color-mix(in srgb,var(--practice-keycap) 55%,transparent);border-color:transparent;box-shadow:none}
.pc-flash{position:absolute;z-index:1;left:50%;top:40%;transform:translate(-50%,-50%);display:flex;gap:var(--spacing-md);animation:pc-flash .9s ${EASE} both}
.pc-flash .pc-kc{min-width:var(--size-practice-flash-key);height:var(--size-practice-flash-key);font-size:var(--text-display);border-radius:var(--radius-xl);padding:0 var(--spacing-md);background:var(--practice-card);box-shadow:0 var(--spacing-xs) 0 var(--practice-line),${SHADOW}}
@keyframes pc-flash{0%{opacity:0;transform:translate(-50%,-50%) scale(.85)}20%{opacity:1;transform:translate(-50%,-50%) scale(1)}75%{opacity:1}100%{opacity:0}}
.pc-mode{position:absolute;left:50%;top:var(--spacing-xs);transform:translateX(-50%);display:flex;align-items:center;gap:var(--spacing-md);font-size:var(--text-body);font-weight:600;padding:var(--spacing-xxs) var(--spacing-md) var(--spacing-xxs) var(--spacing-xs);border-radius:var(--radius-xl);background:var(--practice-card);color:var(--practice-text);box-shadow:0 0 0 var(--size-hairline) var(--practice-line);pointer-events:auto}
.pc-mode button{font:inherit;font-weight:500;border:0;border-radius:var(--radius-xl);padding:var(--spacing-xxs) var(--spacing-md);background:color-mix(in srgb,var(--practice-accent) 30%,transparent);color:var(--practice-ink);cursor:pointer}
.pc-quest{position:absolute;left:var(--spacing-lg);bottom:var(--spacing-lg);width:var(--size-practice-quest);background:var(--practice-card);color:var(--practice-text);border-radius:var(--radius-xl);overflow:hidden;pointer-events:auto;box-shadow:${SHADOW},0 0 0 var(--size-hairline) var(--practice-line);animation:pc-quest .5s ${EASE} both}
@keyframes pc-quest{from{transform:translateY(var(--spacing-xxl));opacity:0}to{transform:none;opacity:1}}
.pc-qh{padding:var(--spacing-md) var(--spacing-md) var(--spacing-sm);background:color-mix(in srgb,var(--practice-accent) 38%,var(--practice-card));color:var(--practice-ink)}
.pc-qh small{display:block;font-size:var(--text-caption);font-weight:700;opacity:.75}
.pc-qh b{font-size:var(--text-title)}
.pc-bar{height:var(--spacing-xs);border-radius:var(--radius-xl);background:color-mix(in srgb,var(--practice-ink) 12%,transparent);margin-top:var(--spacing-sm);overflow:hidden}
.pc-bar i{display:block;height:100%;background:color-mix(in srgb,var(--practice-accent-strong) 80%,var(--practice-ink));border-radius:var(--radius-xl);transition:width .5s ${EASE}}
.pc-quest ol{list-style:none;margin:0;padding:var(--spacing-sm)}
.pc-quest li{display:flex;align-items:center;gap:var(--spacing-sm);padding:var(--spacing-xs) var(--spacing-sm);border-radius:var(--radius-md);font-size:var(--text-subhead);color:var(--practice-mute);transition:background .3s,color .3s}
.pc-quest li.cur{background:color-mix(in srgb,var(--practice-accent) 18%,transparent);color:var(--practice-text);font-weight:600}
.pc-quest li.done{color:var(--practice-text)}
.pc-ck{width:var(--size-checkbox);height:var(--size-checkbox);border-radius:var(--radius-sm);border:var(--size-hairline) solid var(--practice-line);display:grid;place-items:center;flex:none;transition:all .3s ${EASE}}
.pc-quest li.done .pc-ck{background:var(--practice-accent);border-color:var(--practice-accent)}
.pc-quest li.done .pc-ck::after{content:"";width:var(--spacing-xs);height:var(--spacing-sm);border:solid var(--practice-ink);border-width:0 var(--size-tab-indicator) var(--size-tab-indicator) 0;transform:rotate(45deg);animation:pc-pop .35s ${EASE}}
@keyframes pc-pop{0%{transform:rotate(45deg) scale(0)}100%{transform:rotate(45deg) scale(1)}}
.pc-k{margin-left:auto}
.pc-qf{padding:0 var(--spacing-md) var(--spacing-md);font-size:var(--text-body);color:var(--practice-mute);display:flex;justify-content:space-between}
.pc-qf button{font:inherit;border:0;background:none;color:var(--practice-mute);cursor:pointer;padding:0}
.pc-qf button:hover{color:var(--practice-text)}
.pc-hand{position:absolute;z-index:1;filter:drop-shadow(0 var(--spacing-xxs) var(--spacing-xs) color-mix(in srgb,var(--practice-scrim) 50%,transparent))}
.pc-hand path{fill:var(--practice-card);stroke:var(--practice-keycap-ink);stroke-width:1.2}
.pc-example{position:absolute;z-index:1;transform:translate(var(--spacing-xs),-50%);padding:0 var(--spacing-xs);border-radius:var(--radius-xl);border:var(--size-hairline) solid var(--practice-accent-strong);background:color-mix(in srgb,var(--practice-accent) 22%,var(--practice-card));color:var(--practice-ink);font-size:var(--text-caption);font-weight:600;line-height:1.5;white-space:nowrap}
`;

// The chapter list: the same soft amber on the shell's own background, so the
// list and the guide read as one practice place.
export const HUB_CSS = `
.ph-page{min-height:100%;overflow:auto;background:var(--background);color:var(--foreground);display:flex;justify-content:center}
.ph-col{width:min(var(--size-practice-hub),100% - var(--spacing-xxl));padding:calc(var(--spacing-xxxl) * 2) 0 var(--spacing-xxxl)}
.ph-col h1{margin:0;font-size:var(--text-display);font-weight:600;letter-spacing:-.01em}
.ph-lead{margin:var(--spacing-sm) 0 var(--spacing-xl);font-size:var(--text-title);line-height:1.55;color:var(--muted-foreground)}
.ph-list{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:var(--spacing-xs)}
.ph-row{display:flex;align-items:center;gap:var(--spacing-md);padding:var(--spacing-md) var(--spacing-lg);border-radius:var(--radius-xl);transition:background .2s,box-shadow .2s}
.ph-row.on{background:color-mix(in srgb,var(--practice-accent) 9%,transparent);box-shadow:inset 0 0 0 var(--size-hairline) color-mix(in srgb,var(--practice-accent) 45%,transparent)}
.ph-num{flex:none;width:var(--size-icon-button-standard);height:var(--size-icon-button-standard);border-radius:var(--radius-xl);display:grid;place-items:center;font-size:var(--text-title);font-weight:700;background:color-mix(in srgb,var(--foreground) 8%,transparent);color:var(--muted-foreground)}
.ph-num.done{background:var(--practice-accent);color:var(--practice-ink)}
.ph-text{flex:1;min-width:0;display:flex;flex-direction:column;gap:var(--spacing-xxs)}
.ph-text small{font-size:var(--text-caption);color:var(--muted-foreground)}
.ph-text b{font-size:var(--text-title);font-weight:600}
.ph-text span{font-size:var(--text-subhead);color:var(--muted-foreground);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.ph-chip{flex:none;font-size:var(--text-caption);font-weight:600;padding:var(--spacing-xxs) var(--spacing-sm);border-radius:var(--radius-xl);color:var(--muted-foreground);box-shadow:inset 0 0 0 var(--size-hairline) color-mix(in srgb,var(--foreground) 14%,transparent)}
.ph-chip.paused,.ph-chip.changed{color:var(--practice-accent-strong);box-shadow:inset 0 0 0 var(--size-hairline) color-mix(in srgb,var(--practice-accent-strong) 50%,transparent)}
.ph-chip.done{color:var(--practice-accent);box-shadow:none}
.ph-acts{flex:none;display:flex;gap:var(--spacing-xs);align-items:center}
.ph-go{font:inherit;font-size:var(--text-subhead);font-weight:600;border:0;border-radius:var(--radius-lg);padding:var(--spacing-sm) var(--spacing-md);background:var(--practice-accent);color:var(--practice-ink);cursor:pointer}
.ph-row:not(.on) .ph-go{background:color-mix(in srgb,var(--foreground) 8%,transparent);color:var(--foreground)}
.ph-sub{font:inherit;font-size:var(--text-body);border:0;background:none;color:var(--muted-foreground);cursor:pointer;padding:var(--spacing-xs)}
.ph-keys{margin:var(--spacing-lg) var(--spacing-lg) 0;font-size:var(--text-body);color:var(--muted-foreground)}
.ph-note{margin:var(--spacing-xs) var(--spacing-lg) 0;font-size:var(--text-body);color:var(--practice-accent-strong)}
@media (max-width:40rem){.ph-chip{display:none}.ph-row{flex-wrap:wrap}}
`;
