(() => {
  const source = window.StudySource;
  let rows, byType;
  const current = () => {
    const answer = source.catalog();
    if (answer !== rows) {
      rows = answer;
      byType = new Map(rows.map((row) => [row.actor_type, row]));
    }
    return byType;
  };
  const escape = (v) =>
    String(v ?? "").replace(
      /[&<>"']/g,
      (c) =>
        ({
          "&": "&amp;",
          "<": "&lt;",
          ">": "&gt;",
          '"': "&quot;",
          "'": "&#39;",
        })[c],
    );
  function containerCardinality(n) {
    const role = current().get(n.type)?.presentation_role;
    return Number(role?.[0]) === 4 ? role[1] : undefined;
  }
  function sections(rows) {
    const ranks = new Map();
    for (const row of rows)
      if (!ranks.has(row.group)) ranks.set(row.group, row.groupRank ?? 1e6 + ranks.size);
    return [...ranks]
      .sort((a, b) => a[1] - b[1])
      .map(([group]) => ({
        group,
        rows: rows
          .filter((row) => row.group === group)
          .sort((a, b) => String(a.title).localeCompare(String(b.title))),
      }));
  }
  window.ProductCatalog = {
    sections,
    get items() {
      current();
      return rows;
    },
    get byType() {
      return current();
    },
    containerCardinality,
    permissions: (n) => source.permissions(n),
    configFields: (n, draft = n.config) => source.configForm(n, draft),
    readConfig: (form, n) => source.readDraft(form, n),
    get choices() {
      return source.choices;
    },
    escape,
  };
})();
