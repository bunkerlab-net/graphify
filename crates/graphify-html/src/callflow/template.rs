//! Static HTML/CSS/JS template constants for the callflow page.
//!
//! Extracted so the large verbatim blobs do not obscure the logic in the
//! rendering and builder modules. Nothing here contains Rust business logic —
//! it is purely project-agnostic presentation material.

// ── CSS template (fixed, project-agnostic) ─────────────────────────────────

pub(super) static CSS: &str = r":root {
  --bg: #0f172a; --surface: #1e293b; --border: #334155;
  --text: #e2e8f0; --muted: #94a3b8; --accent: #38bdf8;
  --warn: #fbbf24; --err: #f87171; --ok: #34d399;
}
* { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: 'Segoe UI', system-ui, -apple-system, sans-serif; background: var(--bg); color: var(--text); line-height: 1.7; }
.container { max-width: 1200px; margin: 0 auto; padding: 40px 24px; }
h1 { font-size: 2.4rem; margin-bottom: 8px; background: linear-gradient(135deg, var(--accent), #a78bfa); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
h2 { font-size: 1.7rem; margin: 48px 0 16px; padding-bottom: 8px; border-bottom: 2px solid var(--accent); }
h3 { font-size: 1.25rem; margin: 32px 0 12px; color: var(--accent); }
h4 { font-size: 1.05rem; margin: 20px 0 8px; color: var(--warn); }
p { margin: 8px 0; color: var(--muted); }
.subtitle { color: var(--muted); font-size: 1.1rem; margin-bottom: 32px; }
.mermaid { background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 24px; margin: 20px 0; overflow-x: auto; position: relative; }
.mermaid.is-enhanced { padding: 0; overflow: hidden; min-height: 260px; }
.mermaid-viewport { padding: 54px 24px 24px; overflow: hidden; cursor: grab; touch-action: none; min-height: 260px; }
.mermaid-viewport.is-dragging { cursor: grabbing; }
.mermaid-viewport svg { max-width: none !important; height: auto; transform-origin: 0 0; transition: transform 120ms ease; }
.mermaid-toolbar { position: absolute; top: 10px; right: 10px; z-index: 3; display: flex; align-items: center; gap: 6px; padding: 6px; background: rgba(15,23,42,0.92); border: 1px solid var(--border); border-radius: 8px; box-shadow: 0 8px 24px rgba(0,0,0,0.28); }
.mermaid-toolbar button, .mermaid-toolbar .zoom-level { height: 28px; min-width: 32px; border: 1px solid var(--border); border-radius: 6px; background: #1e293b; color: var(--text); font: 600 0.78rem system-ui, sans-serif; display: inline-flex; align-items: center; justify-content: center; }
.mermaid-toolbar button { cursor: pointer; }
.mermaid-toolbar button:hover { border-color: var(--accent); color: var(--accent); }
.mermaid-toolbar .zoom-level { min-width: 52px; color: var(--muted); background: transparent; }
.call-table { width: 100%; border-collapse: collapse; margin: 16px 0; font-size: 0.92rem; }
.call-table th { background: #1a2744; color: var(--accent); text-align: left; padding: 10px 14px; border: 1px solid var(--border); }
.call-table td { padding: 8px 14px; border: 1px solid var(--border); vertical-align: top; }
.call-table tr:nth-child(even) { background: rgba(255,255,255,0.02); }
.tag { display: inline-block; padding: 2px 8px; border-radius: 4px; font-size: 0.8rem; font-weight: 600; }
.tag-async { background: #7c3aed33; color: #a78bfa; }
.tag-class { background: #05966933; color: var(--ok); }
.tag-func { background: #2563eb33; color: var(--accent); }
.tag-cmd { background: #d9770633; color: var(--warn); }
.tag-endpoint { background: #dc262633; color: var(--err); }
.tag-hook { background: #db277733; color: #f472b6; }
.card { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; padding: 20px; margin: 16px 0; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(340px, 1fr)); gap: 16px; margin: 16px 0; }
.arrow-chain { font-family: 'Fira Code', monospace; font-size: 0.85rem; color: var(--accent); padding: 10px; background: rgba(56,189,248,0.06); border-radius: 6px; }
code { font-family: 'Fira Code', 'Cascadia Code', monospace; background: rgba(255,255,255,0.06); padding: 1px 6px; border-radius: 3px; font-size: 0.88em; }
ul, ol { margin: 8px 0 8px 24px; color: var(--muted); }
li { margin: 4px 0; }
a { color: var(--accent); }
hr { border: none; border-top: 1px solid var(--border); margin: 40px 0; }
.nav { position: sticky; top: 0; background: var(--bg); z-index: 10; padding: 12px 0; border-bottom: 1px solid var(--border); display: flex; gap: 20px; flex-wrap: wrap; font-size: 0.9rem; }
.nav a { text-decoration: none; }
.nav a:hover { text-decoration: underline; }
@media (max-width: 768px) { .container { padding: 16px; } h1 { font-size: 1.8rem; } }";

// ── JS footer (interactive zoom/pan for every .mermaid block) ───────────────

pub(super) static JS_FOOTER: &str = r#"<script>
(function () {
  const mermaidConfig = {
    startOnLoad: false,
    theme: 'dark',
    securityLevel: 'loose',
    flowchart: { htmlLabels: true, useMaxWidth: true },
    themeVariables: {
      primaryColor: '#1e293b',
      primaryTextColor: '#e2e8f0',
      primaryBorderColor: '#38bdf8',
      secondaryColor: '#0f172a',
      tertiaryColor: '#334155',
      lineColor: '#64748b',
      textColor: '#e2e8f0',
    }
  };

  mermaid.initialize(mermaidConfig);

  function clamp(value, min, max) {
    return Math.min(max, Math.max(min, value));
  }

  function enhanceMermaidDiagrams() {
    document.querySelectorAll('.mermaid').forEach((container) => {
      if (container.dataset.zoomReady === 'true') return;
      const svg = container.querySelector('svg');
      if (!svg) return;

      container.dataset.zoomReady = 'true';
      container.classList.add('is-enhanced');

      const viewport = document.createElement('div');
      viewport.className = 'mermaid-viewport';
      svg.parentNode.insertBefore(viewport, svg);
      viewport.appendChild(svg);

      const toolbar = document.createElement('div');
      toolbar.className = 'mermaid-toolbar';
      toolbar.innerHTML = [
        '<button type="button" data-action="zoom-out" title="Zoom out">-</button>',
        '<span class="zoom-level" data-role="level">100%</span>',
        '<button type="button" data-action="zoom-in" title="Zoom in">+</button>',
        '<button type="button" data-action="fit" title="Fit width">Fit</button>',
        '<button type="button" data-action="reset" title="Reset view">Reset</button>'
      ].join('');
      container.insertBefore(toolbar, viewport);

      const state = { scale: 1, x: 0, y: 0, dragging: false, startX: 0, startY: 0, originX: 0, originY: 0 };
      const level = toolbar.querySelector('[data-role="level"]');

      function applyTransform() {
        svg.style.transform = `translate(${state.x}px, ${state.y}px) scale(${state.scale})`;
        level.textContent = `${Math.round(state.scale * 100)}%`;
      }

      function zoomBy(delta) {
        state.scale = clamp(state.scale + delta, 0.25, 3);
        applyTransform();
      }

      function reset() {
        state.scale = 1;
        state.x = 0;
        state.y = 0;
        applyTransform();
      }

      function fitWidth() {
        const rawWidth = svg.viewBox && svg.viewBox.baseVal && svg.viewBox.baseVal.width
          ? svg.viewBox.baseVal.width
          : svg.getBoundingClientRect().width / state.scale;
        if (!rawWidth) {
          reset();
          return;
        }
        state.scale = clamp((viewport.clientWidth - 48) / rawWidth, 0.25, 1.4);
        state.x = 0;
        state.y = 0;
        applyTransform();
      }

      toolbar.addEventListener('click', (event) => {
        const button = event.target.closest('button[data-action]');
        if (!button) return;
        const action = button.dataset.action;
        if (action === 'zoom-in') zoomBy(0.15);
        if (action === 'zoom-out') zoomBy(-0.15);
        if (action === 'fit') fitWidth();
        if (action === 'reset') reset();
      });

      viewport.addEventListener('wheel', (event) => {
        if (!event.ctrlKey && !event.metaKey) return;
        event.preventDefault();
        zoomBy(event.deltaY < 0 ? 0.1 : -0.1);
      }, { passive: false });

      viewport.addEventListener('pointerdown', (event) => {
        if (event.button !== 0) return;
        state.dragging = true;
        state.startX = event.clientX;
        state.startY = event.clientY;
        state.originX = state.x;
        state.originY = state.y;
        viewport.classList.add('is-dragging');
        viewport.setPointerCapture(event.pointerId);
      });

      viewport.addEventListener('pointermove', (event) => {
        if (!state.dragging) return;
        state.x = state.originX + event.clientX - state.startX;
        state.y = state.originY + event.clientY - state.startY;
        applyTransform();
      });

      function endDrag(event) {
        if (!state.dragging) return;
        state.dragging = false;
        viewport.classList.remove('is-dragging');
        if (viewport.hasPointerCapture(event.pointerId)) {
          viewport.releasePointerCapture(event.pointerId);
        }
      }

      viewport.addEventListener('pointerup', endDrag);
      viewport.addEventListener('pointercancel', endDrag);
      applyTransform();
    });
  }

  function renderMermaid() {
    const result = mermaid.run
      ? mermaid.run({ querySelector: '.mermaid' })
      : Promise.resolve();
    Promise.resolve(result)
      .then(enhanceMermaidDiagrams)
      .catch((error) => {
        console.error('Mermaid render failed:', error);
        enhanceMermaidDiagrams();
      });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', renderMermaid);
  } else {
    renderMermaid();
  }
})();
</script>"#;
