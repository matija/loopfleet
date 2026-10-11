import type { ReactNode } from "react";
import { ChecklistIcon, DiffIcon, PlayIcon } from "./Icon";

export function ProjectStart({ hasProjects, children }: { hasProjects: boolean; children: ReactNode }) {
  return (
    <section className="project-start" aria-labelledby="project-start-title">
      <div className="project-start__intro">
        <div className="project-start__mark" aria-hidden="true">
          <svg viewBox="0 0 256 256" fill="none">
            <path d="M106.63 152.13l-8.69 9.81a48 48 0 1 1 0-67.88l60.12 67.88a48 48 0 1 0 0-67.88l-8.69 9.81" stroke="currentColor" strokeWidth="16" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </div>
        <p className="project-start__eyebrow">Your coding workspace</p>
        <h1 id="project-start-title">{hasProjects ? "Pick up where you left off" : "From a plan to working code"}</h1>
        <p className="project-start__description">
          {hasProjects ? "Open a project to continue your plan, launch agents, and review their work." : "Bring a repository. Make a plan. Let your agents take it from there."}
        </p>
        <div className="project-start__actions">{children}</div>
        {!hasProjects && <p className="project-start__hint">Choose a local Git repository to get started.</p>}
      </div>
      <ol className="project-start__steps" aria-label="How Loopfleet works">
        <li>
          <div className="project-start__step-head"><ChecklistIcon size={20} /><span>01</span></div>
          <h2>Make a plan</h2>
          <p>Describe what you want to build and turn it into tasks.</p>
        </li>
        <li>
          <div className="project-start__step-head"><PlayIcon size={20} /><span>02</span></div>
          <h2>Run your agents</h2>
          <p>Launch tasks in isolated worktrees and follow their progress.</p>
        </li>
        <li>
          <div className="project-start__step-head"><DiffIcon size={20} /><span>03</span></div>
          <h2>Review &amp; merge</h2>
          <p>Inspect the changes and bring the work you approve into your project.</p>
        </li>
      </ol>
    </section>
  );
}
