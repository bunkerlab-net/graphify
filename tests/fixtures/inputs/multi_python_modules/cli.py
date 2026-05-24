"""Command-line interface for the task management system."""
from __future__ import annotations
import sys
from models import Priority
from service import TaskService


def _parse_priority(s: str) -> Priority:
    mapping = {p.name.lower(): p for p in Priority}
    p = mapping.get(s.lower())
    if p is None:
        raise ValueError(f"Unknown priority: {s}. Choose from: {list(mapping)}")
    return p


def run_demo() -> None:
    svc = TaskService()

    # Register users
    alice = svc.register_user("alice", "alice@example.com")
    bob = svc.register_user("bob", "bob@example.com")

    # Create project
    proj = svc.create_project("Website Relaunch", alice.id)

    # Add tasks
    t1 = svc.create_task(
        "Design landing page",
        "Create mockups for the new landing page",
        proj.id,
        Priority.HIGH,
        alice.id,
    )
    t2 = svc.create_task(
        "Backend API",
        "Implement REST endpoints",
        proj.id,
        Priority.CRITICAL,
        bob.id,
    )
    t3 = svc.create_task(
        "Write docs",
        "Update README and API docs",
        proj.id,
        Priority.LOW,
    )

    # Complete one
    svc.complete_task(t1.id)

    # Show dashboard
    for user in [alice, bob]:
        dash = svc.dashboard(user.id)
        print(f"\n=== Dashboard for {dash['user']} ===")
        print(f"  Total tasks:   {dash['total']}")
        print(f"  In progress:   {dash['in_progress']}")
        print(f"  Completed:     {dash['done']}")
        print(f"  Overdue:       {dash['overdue']}")

    project = svc.get_project(proj.id)
    if project:
        print(f"\nProject '{project.name}' completion: {project.completion_ratio():.0%}")


if __name__ == "__main__":
    run_demo()
