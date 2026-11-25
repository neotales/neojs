import { createTemplateSet, type TemplateSet } from "vael:template";
import type { Task, TaskFilter } from "./db.ts";

/** View model passed to the `home` SSR template. */
export interface HomeViewData {
  heading: string;
  filter: TaskFilter;
  tasks: Task[];
  totalCount: number;
  flash: string;
}

const LAYOUT_TEMPLATE = `{{define "layout"}}<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{{block "title" .}}VaelJS Tasks{{end}}</title>
  <link rel="stylesheet" href="/assets/style.css">
</head>
<body>
  <main class="container">
    {{block "content" .}}{{end}}
  </main>
</body>
</html>{{end}}`;

const TASK_ITEM_PARTIAL = `<li class="task-item{{if .done}} is-done{{end}}">
  <div class="task-main">
    <span class="badge">{{if .done}}DONE{{else}}OPEN{{end}}</span>
    <span class="task-title">{{.title | trim}}</span>
  </div>
  <div class="task-actions">
    <form method="post" action="/tasks/{{.id}}/toggle">
      <button type="submit" class="btn btn-secondary">{{if .done}}Reopen{{else}}Complete{{end}}</button>
    </form>
    <form method="post" action="/tasks/{{.id}}/delete">
      <button type="submit" class="btn btn-danger">Delete</button>
    </form>
  </div>
</li>`;

const HOME_TEMPLATE = `{{define "title"}}{{.heading | default "vaeljs task board" | trim | title}} · VaelJS{{end}}
{{define "content"}}
<header class="page-header">
  <div>
    <p class="eyebrow">ZERO-FRAMEWORK VAELJS SSR</p>
    <h1>{{.heading | default "vaeljs task board" | trim | title}}</h1>
  </div>
  <span class="status-pill" data-filter="{{.filter | default "all" | trim | upper}}">FILTER: {{.filter | default "all" | trim | upper}} ({{.totalCount}})</span>
</header>

{{with (.flash | default "" | trim)}}
<div class="flash" role="status">{{.}}</div>
{{end}}

<form class="create-form" method="post" action="/tasks">
  <input type="text" name="title" placeholder="Add a task..." required autocomplete="off">
  <button type="submit" class="btn btn-primary">Add Task</button>
</form>

<nav class="filters" aria-label="Filter tasks">
  {{range .filters}}<a href="/?status={{.value}}" class="filter-link{{if .active}} active{{end}}">{{.label | title}}</a>{{end}}
</nav>

{{if .tasks}}
<ul class="task-list">
  {{range .tasks}}{{template "task_item" .}}{{end}}
</ul>
{{else}}
<p class="empty-state">No tasks match the {{.filter | default "all" | trim | upper}} filter.</p>
{{end}}
{{end}}
{{template "layout" .}}`;

/**
 * Build the HTML-escaping Go-style template set with Sprig helpers enabled.
 */
export function createAppTemplates(): TemplateSet {
  return createTemplateSet({ sprig: true, htmlEscape: true })
    .add("layout", LAYOUT_TEMPLATE)
    .add("task_item", TASK_ITEM_PARTIAL)
    .add("home", HOME_TEMPLATE);
}

/**
 * Render the SSR home page using the pre-compiled template set.
 */
export function renderHomePage(templates: TemplateSet, data: HomeViewData): string {
  const filters: ReadonlyArray<{ value: TaskFilter; label: string; active: boolean }> = [
    { value: "all", label: "all", active: data.filter === "all" },
    { value: "open", label: "open", active: data.filter === "open" },
    { value: "done", label: "done", active: data.filter === "done" },
  ];
  return templates.render("home", { ...data, filters });
}
