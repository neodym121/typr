// Icons: Lucide (lucide.dev) v1.48.0, the SVG files unchanged in src/icons
// (license there). They are bundled with the app, nothing is loaded from the
// internet. Markup puts `<i data-icon="name">` where an icon goes and
// renderIcons() swaps it for the SVG; code calls icon().

const FILES = import.meta.glob<string>("./icons/*.svg", { query: "?raw", import: "default", eager: true });

const ICONS = new Map(
  Object.entries(FILES).map(([path, svg]) => [path.slice("./icons/".length, -".svg".length), svg]),
);

/** The icon `name` (its file name in src/icons), `size` px square, drawn in the text colour */
export function icon(name: string, size = 16): SVGSVGElement {
  const template = document.createElement("template");
  const source = ICONS.get(name);
  if (!source) console.error(`No icon "${name}" in src/icons`);
  template.innerHTML = source ?? '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"></svg>';
  const svg = template.content.querySelector("svg")!;
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("aria-hidden", "true");
  return svg;
}

/** Replaces each `<i data-icon="name" data-size="16">` under `root` with its
 *  icon; the placeholder's classes go along */
export function renderIcons(root: ParentNode = document) {
  root.querySelectorAll<HTMLElement>("i[data-icon]").forEach((placeholder) => {
    const svg = icon(placeholder.dataset.icon!, Number(placeholder.dataset.size) || 16);
    svg.classList.add(...placeholder.classList);
    placeholder.replaceWith(svg);
  });
}
