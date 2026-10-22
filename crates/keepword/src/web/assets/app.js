// Keepword's dashboard works without this script; it only makes it nicer.
"use strict";

// Ask before forms that delete or stop things; say a slow one is working.
document.addEventListener("submit", (e) => {
  const form = e.target;
  if (form.dataset.confirm && !window.confirm(form.dataset.confirm)) {
    e.preventDefault();
    return;
  }
  const button = e.submitter || form.querySelector("button[type=submit]");
  if (button && button.dataset.busy) {
    // After the form is on its way: a disabled button isn't submitted.
    setTimeout(() => {
      button.disabled = true;
      button.textContent = button.dataset.busy;
    }, 0);
  }
});

// Exact times in the reader's own time zone, on hover.
for (const t of document.querySelectorAll("time[datetime]")) {
  const d = new Date(t.getAttribute("datetime"));
  if (!Number.isNaN(d.getTime())) t.title = d.toLocaleString();
}

// Filter a list as you type.
for (const input of document.querySelectorAll("input[data-filter]")) {
  input.addEventListener("input", () => {
    const q = input.value.trim().toLowerCase();
    for (const row of document.querySelectorAll(input.dataset.filter)) {
      const text = row.dataset.search || row.textContent.toLowerCase();
      row.hidden = q !== "" && !text.includes(q);
    }
  });
}

// Back to the dashboard once a restart is done.
const later = document.querySelector("[data-reload-after]");
if (later) {
  setTimeout(() => {
    window.location.href = later.getAttribute("href");
  }, 1000 * Number(later.dataset.reloadAfter));
}
