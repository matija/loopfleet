import type { ReactNode } from "react";

export function ProjectStart({ children }: { children: ReactNode }) {
  return (
    <section className="project-start" aria-labelledby="project-start-title">
      <h1 id="project-start-title">Projects</h1>
      <div className="project-start__actions">{children}</div>
    </section>
  );
}
