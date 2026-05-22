"""Business logic layer for task management."""
from __future__ import annotations
from typing import Optional
from models import User, Task, Project, Priority, Status
from storage import UserStore, TaskStore, ProjectStore


class TaskService:
    """Orchestrates task operations and enforces business rules."""

    def __init__(self) -> None:
        self._users = UserStore()
        self._tasks = TaskStore()
        self._projects = ProjectStore(self._users, self._tasks)

    # --- user operations ---

    def register_user(self, username: str, email: str) -> User:
        existing = self._users.find_by_email(email)
        if existing:
            raise ValueError(f"Email already registered: {email}")
        return self._users.create(username, email)

    def get_user(self, user_id: int) -> Optional[User]:
        return self._users.get(user_id)

    # --- project operations ---

    def create_project(self, name: str, owner_id: int) -> Project:
        owner = self._users.get(owner_id)
        if owner is None:
            raise ValueError(f"User not found: {owner_id}")
        return self._projects.create(name, owner)

    def get_project(self, project_id: int) -> Optional[Project]:
        return self._projects.get(project_id)

    # --- task operations ---

    def create_task(
        self,
        title: str,
        description: str,
        project_id: int,
        priority: Priority = Priority.MEDIUM,
        assignee_id: Optional[int] = None,
    ) -> Task:
        project = self._projects.get(project_id)
        if project is None:
            raise ValueError(f"Project not found: {project_id}")
        assignee = self._users.get(assignee_id) if assignee_id else None
        task = self._tasks.create(title, description, assignee, priority)
        project.add_task(task)
        return task

    def assign_task(self, task_id: int, user_id: int) -> Task:
        task = self._tasks.get(task_id)
        if task is None:
            raise ValueError(f"Task not found: {task_id}")
        user = self._users.get(user_id)
        if user is None:
            raise ValueError(f"User not found: {user_id}")
        task.assign_to(user)
        return task

    def complete_task(self, task_id: int) -> Task:
        task = self._tasks.get(task_id)
        if task is None:
            raise ValueError(f"Task not found: {task_id}")
        task.complete()
        return task

    def dashboard(self, user_id: int) -> dict:
        user = self._users.get(user_id)
        if user is None:
            raise ValueError(f"User not found: {user_id}")
        assigned = self._tasks.by_assignee(user)
        return {
            "user": user.display_name(),
            "total": len(assigned),
            "in_progress": [t.title for t in assigned if t.status == Status.IN_PROGRESS],
            "done": sum(1 for t in assigned if t.status == Status.DONE),
            "overdue": [t.title for t in assigned if t.is_overdue()],
        }
