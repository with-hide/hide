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
.pc-scrim{position:absolute;inset:0;width:100%;height:100%;pointer-events:none}
.pc-scrim-fill{fill:color-mix(in srgb,var(--practice-scrim) 58%,transparent)}
.pc-spot{position:absolute;border-radius:var(--radius-xl);box-shadow:0 0 0 calc(var(--size-tab-indicator) * 1.5) var(--practice-accent),0 0 0 var(--spacing-md) color-mix(in srgb,var(--practice-accent) 28%,transparent);
  transition:left .5s ${EASE},top .5s ${EASE},width .5s ${EASE},height .5s ${EASE}}
.pc-spot::after{content:"";position:absolute;inset:calc(var(--size-tab-indicator) * -1.5);border-radius:var(--radius-xl);border:var(--size-tab-indicator) solid var(--practice-accent);animation:pc-pulse 2.6s ease-out infinite}
@keyframes pc-pulse{0%{opacity:.6;transform:scale(1)}100%{opacity:0;transform:scale(1.04)}}
.pc-zdim{position:absolute;inset:0;background:color-mix(in srgb,var(--practice-scrim) 32%,transparent);animation:pc-in .4s both}
.pc-zone{position:absolute;border-radius:var(--radius-xl);border:var(--size-tab-indicator) dashed;animation:pc-zone .5s ${EASE} both}
${ZONES}
/* A zone's tag rests on the zone's bottom edge, where a list ends and a terminal is empty, so it covers no row and no terminal text. */
.pc-tag{position:absolute;left:50%;bottom:var(--spacing-xs);transform:translateX(-50%);padding:var(--spacing-xs) var(--spacing-md);border-radius:var(--radius-xl);font-weight:700;font-size:var(--text-body);white-space:nowrap;box-shadow:${SHADOW};text-align:center;line-height:1.35;color:var(--practice-ink)}
/* The project zone is the tool rail alone while the lane is closed, narrower than its tag, so the tag ends where the screen does. */
.pc-zone.project .pc-tag{left:auto;right:0;transform:none}
/* The left list fills its zone to the bottom, so its tag sits on the zone's top edge, in the title row's empty stretch. */
.pc-zone.view .pc-tag{left:auto;right:0;bottom:auto;top:0;transform:translateY(-100%)}
.pc-tag small{display:inline;margin-left:var(--spacing-sm);font-size:var(--text-caption);font-weight:500;opacity:.8}
@keyframes pc-zone{from{opacity:0;transform:scale(.98)}to{opacity:1;transform:scale(1)}}
@keyframes pc-in{from{opacity:0}to{opacity:1}}
.pc-kick{font-size:var(--text-caption);font-weight:600;letter-spacing:.02em;color:color-mix(in srgb,var(--practice-accent-strong) 60%,var(--practice-ink));display:flex;justify-content:space-between;gap:var(--spacing-sm)}
.pc-kick span:last-child{flex:none;white-space:nowrap}
.pc-via{margin-top:var(--spacing-xs);font-size:var(--text-body);color:var(--practice-mute);display:flex;gap:var(--spacing-sm);align-items:center}
.pc-keys{display:inline-flex;gap:var(--spacing-xxs)}
.pc-row{display:flex;align-items:center;gap:var(--spacing-sm);margin-top:var(--spacing-md)}
.pc-dots{display:flex;gap:var(--spacing-xs);margin-right:auto;min-width:0;overflow:hidden}
.pc-dots i{width:var(--size-tab-status-dot);height:var(--size-tab-status-dot);border-radius:var(--radius-xl);background:var(--practice-line);transition:all .3s}
.pc-dots i.on{width:var(--spacing-lg);background:var(--practice-accent)}
.pc-dots i.done{background:color-mix(in srgb,var(--practice-accent) 55%,var(--practice-line))}
.pc-go{font:inherit;font-size:var(--text-subhead);font-weight:600;border:0;border-radius:var(--radius-lg);padding:var(--spacing-sm) var(--spacing-md);background:var(--practice-accent);color:var(--practice-ink);cursor:pointer;display:flex;gap:var(--spacing-sm);align-items:center;white-space:nowrap;flex:none}
.pc-back{font:inherit;font-size:var(--text-body);border:0;background:none;color:var(--practice-mute);cursor:pointer;padding:var(--spacing-xs);white-space:nowrap;flex:none}
.pc-back:hover,.pc-exit:hover{color:var(--practice-text)}
.pc-go.pc-sub{background:color-mix(in srgb,var(--practice-ink) 8%,transparent)}
.pc-kc{display:inline-flex;align-items:center;justify-content:center;min-width:var(--size-control-compact);height:var(--size-control-compact);padding:0 var(--spacing-xs);border-radius:var(--radius-sm);font-size:var(--text-body);font-weight:700;background:var(--practice-keycap);color:var(--practice-keycap-ink);box-shadow:0 var(--size-hairline) 0 var(--practice-line);border:var(--size-hairline) solid var(--practice-line)}
.pc-go .pc-kc{background:color-mix(in srgb,var(--practice-keycap) 55%,transparent);border-color:transparent;box-shadow:none}
.pc-mark{position:absolute;z-index:60;box-sizing:border-box;border-radius:var(--radius-sm);border:calc(var(--size-tab-indicator) * 1.5) solid var(--practice-accent);background:color-mix(in srgb,var(--practice-accent) 14%,transparent);pointer-events:none;transition:left .5s ${EASE},top .5s ${EASE},width .5s ${EASE},height .5s ${EASE}}
.pc-call{position:absolute;z-index:61;box-sizing:border-box;display:flex;align-items:center;padding:0 var(--spacing-md);height:var(--size-control-compact);border-radius:var(--radius-md);background:var(--practice-accent);color:var(--practice-ink);font-size:var(--text-body);font-weight:700;white-space:nowrap;box-shadow:${SHADOW};pointer-events:none;transition:left .5s ${EASE},top .5s ${EASE}}
.pc-mode{position:absolute;left:50%;top:var(--spacing-xs);transform:translateX(-50%);display:flex;align-items:center;gap:var(--spacing-md);font-size:var(--text-body);font-weight:600;padding:var(--spacing-xxs) var(--spacing-xs);white-space:nowrap;border-radius:var(--radius-xl);background:var(--practice-card);color:var(--practice-text);box-shadow:0 0 0 var(--size-hairline) var(--practice-line);pointer-events:auto}
.pc-mode button{font:inherit;font-weight:500;border:0;border-radius:var(--radius-xl);padding:var(--spacing-xxs) var(--spacing-md);background:color-mix(in srgb,var(--practice-accent) 30%,transparent);color:var(--practice-ink);cursor:pointer}
.pc-mode button.pc-leave{background:var(--practice-accent);font-weight:600}

.pc-card{position:absolute;z-index:61;box-sizing:border-box;width:var(--size-practice-bubble);max-width:calc(100vw - var(--spacing-xxl));padding:var(--spacing-lg);border-radius:var(--radius-xl);background:var(--practice-card);color:var(--practice-text);box-shadow:${SHADOW},0 0 0 var(--size-hairline) var(--practice-line);pointer-events:auto;transition:left .5s ${EASE},top .5s ${EASE}}
.pc-card:lang(ko){word-break:keep-all}
.pc-card-text{animation:pc-card .35s ${EASE} both}
@keyframes pc-card{from{opacity:0;transform:translateY(var(--spacing-sm))}to{opacity:1;transform:none}}
.pc-card h3{margin:var(--spacing-xxs) 0;font-size:var(--text-headline);line-height:1.35;font-weight:600}
.pc-body{display:flex;flex-direction:column;gap:var(--spacing-xs);margin-top:var(--spacing-xs)}
.pc-card p{margin:0;font-size:var(--text-subhead);line-height:1.6;color:var(--practice-mute);text-wrap:pretty}
.pc-card p b{font-weight:700;color:var(--practice-text)}
.pc-card p.pc-item{position:relative;padding-left:var(--spacing-lg)}
.pc-card p.pc-item::before{content:"";position:absolute;left:var(--spacing-xs);top:.7em;width:var(--spacing-xs);height:var(--spacing-xs);border-radius:var(--radius-xl);background:var(--practice-accent)}
.pc-card p.pc-quote{padding:var(--spacing-xs) var(--spacing-md);border-radius:var(--radius-md);border-left:var(--spacing-xxs) solid var(--practice-accent);background:color-mix(in srgb,var(--practice-accent) 14%,transparent);color:var(--practice-text);font-weight:600}
`;

// The chapter list: cards on the shell's own background, chosen in the soft
// amber the guide uses, so the list and the guide read as one practice place.
export const HUB_CSS = `
.ph-page{min-height:100%;overflow:auto;background:var(--background);color:var(--foreground);display:flex;justify-content:center}
.ph-col{width:min(var(--size-practice-hub),100% - var(--spacing-xxl));padding:calc(var(--spacing-xxxl) * 2) 0 var(--spacing-xxxl)}
.ph-head{display:flex;align-items:center;justify-content:space-between;gap:var(--spacing-md)}
.ph-col h1{margin:0;font-size:var(--text-display);font-weight:600;letter-spacing:-.01em}
.ph-lead{margin:var(--spacing-sm) 0 0;font-size:var(--text-title);line-height:1.55;color:var(--muted-foreground)}
.ph-again{margin:var(--spacing-xxs) 0 var(--spacing-xl);font-size:var(--text-subhead);color:var(--muted-foreground)}
.ph-grid{list-style:none;margin:0;padding:0;display:grid;grid-template-columns:repeat(auto-fill,minmax(calc(var(--size-practice-hub) / 3 - var(--spacing-md)),1fr));gap:var(--spacing-md)}
.ph-cell{opacity:0;transform:translateY(var(--spacing-md));animation:ph-deal .45s cubic-bezier(.2,.8,.2,1) forwards}
@keyframes ph-deal{to{opacity:1;transform:none}}
@media (prefers-reduced-motion:reduce){.ph-cell{animation:none;opacity:1;transform:none}}
.ph-card.ph-card{height:100%;flex-direction:column;align-items:stretch;gap:0;padding:0;overflow:hidden;border-radius:var(--radius-xl);box-shadow:inset 0 0 0 var(--size-hairline) var(--border);transition:box-shadow .2s,transform .2s}
.ph-card.ph-card:hover{background:transparent}
.ph-card.on{box-shadow:inset 0 0 0 var(--size-hairline) var(--practice-accent),0 0 0 var(--spacing-xxs) color-mix(in srgb,var(--practice-accent) 22%,transparent);transform:translateY(calc(var(--spacing-xxs) * -1))}
.ph-card .pv{border-bottom:var(--size-hairline) solid var(--border)}
.ph-meta{display:flex;flex-direction:column;gap:var(--spacing-xxs);padding:var(--spacing-sm) var(--spacing-md) var(--spacing-md);min-width:0}
.ph-top{display:flex;align-items:center;justify-content:space-between;gap:var(--spacing-xs)}
.ph-top small{font-size:var(--text-caption);color:var(--muted-foreground)}
.ph-meta b{font-size:var(--text-title);font-weight:600}
.ph-sum{font-size:var(--text-body);line-height:1.45;color:var(--muted-foreground);display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;white-space:normal}
.ph-chip{flex:none;font-size:var(--text-micro);font-weight:600;padding:0 var(--spacing-xs);border-radius:var(--radius-xl);color:var(--muted-foreground);box-shadow:inset 0 0 0 var(--size-hairline) color-mix(in srgb,var(--foreground) 14%,transparent)}
.ph-chip.paused,.ph-chip.changed{color:var(--practice-accent-strong);box-shadow:inset 0 0 0 var(--size-hairline) color-mix(in srgb,var(--practice-accent-strong) 50%,transparent)}
.ph-chip.done{color:var(--practice-accent);box-shadow:none}
.ph-keys{margin:var(--spacing-lg) 0 0;font-size:var(--text-body);color:var(--muted-foreground)}
.ph-note{margin:var(--spacing-xs) 0 0;font-size:var(--text-body);color:var(--practice-accent-strong)}
.ph-detail.ph-detail{width:min(var(--size-practice-hub) * .75,100% - var(--spacing-xxl))}
.ph-detail.ph-guide{width:min(var(--size-practice-hub),100% - var(--spacing-xxl))}
.ph-detail .pv{flex:none}
.ph-body{display:flex;flex-direction:column;gap:var(--spacing-xxs);padding:var(--spacing-lg) var(--spacing-lg) 0;overflow:auto}
.ph-body small{font-size:var(--text-caption);color:var(--muted-foreground)}
.ph-title{margin:0;font-size:var(--text-headline);font-weight:600}
.ph-desc{margin:0;font-size:var(--text-subhead);line-height:1.5;color:var(--muted-foreground)}
.ph-learn-head{margin:var(--spacing-md) 0 var(--spacing-xxs);font-size:var(--text-caption);font-weight:600;color:var(--muted-foreground)}
.ph-learn{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:var(--spacing-xxs)}
.ph-learn li{display:flex;align-items:center;justify-content:space-between;gap:var(--spacing-sm);padding:var(--spacing-xs) var(--spacing-sm);border-radius:var(--radius-md);background:var(--secondary);font-size:var(--text-body)}
.pf{list-style:none;margin:0;padding:var(--spacing-lg) var(--spacing-lg) 0;display:grid;grid-template-columns:repeat(auto-fit,minmax(calc(var(--size-practice-hub) / 5),1fr));gap:var(--spacing-md)}
.pf li{display:flex;flex-direction:column;gap:var(--spacing-xs);opacity:var(--opacity-dimmed);transition:opacity .3s}
.pf li.on{opacity:1}
.pf .pv{border-radius:var(--radius-md);box-shadow:inset 0 0 0 var(--size-hairline) var(--border)}
.pf li.on .pv{box-shadow:inset 0 0 0 var(--size-hairline) var(--practice-accent),0 0 0 var(--spacing-xxs) color-mix(in srgb,var(--practice-accent) 22%,transparent)}
.pf li>span{display:flex;gap:var(--spacing-xs);font-size:var(--text-body);line-height:1.4;color:var(--foreground)}
.pf li>span>b{flex:none;display:grid;place-items:center;width:var(--spacing-lg);height:var(--spacing-lg);border-radius:var(--radius-xl);font-size:var(--text-micro);background:var(--practice-accent);color:var(--practice-ink)}
.ph-foot.ph-foot{padding:var(--spacing-lg)}
`;
