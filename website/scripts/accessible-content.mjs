// Static focusability keeps overflowing examples keyboard-readable even
// without JavaScript and in engines that do not auto-focus scroll containers.
function visitElements(node, tag, enhance) {
  if (node.type === 'element' && node.tagName === tag) enhance(node);
  for (const child of node.children || []) visitElements(child, tag, enhance);
}

export const accessibleCode = {
  name: 'keyboard-scrollable-code',
  hooks: {
    postprocessRenderedBlock({ renderData, codeBlock }) {
      visitElements(renderData.blockAst, 'pre', node => {
        node.properties.tabIndex = 0;
        node.properties.role = 'region';
        node.properties.ariaLabel = `${codeBlock.language || 'Text'} code example`;
      });
    },
  },
};

export const accessibleTables = {
  name: 'keyboard-scrollable-tables',
  element: {
    filter: ['table'],
    visit(node) {
      return {
        type: 'element',
        tagName: 'div',
        properties: {
          class: 'rb-table-scroll',
          tabindex: 0,
          role: 'region',
          'aria-label': 'Scrollable reference table',
        },
        children: [node],
      };
    },
  },
};
