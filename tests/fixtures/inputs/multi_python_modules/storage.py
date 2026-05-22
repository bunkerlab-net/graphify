"""In-memory storage layer for task management."""
from __future__ import annotations
from typing import Optional
from models import User, Task, Project, Priority, Status


class UserStore:
    """Simple in-memory user repository."""

    def __init__(self) -> None:
        self._users: dict[int, User] = {}
        self._next_id = 1

    def create(self, username: str, email: str) -> User:
        user = User(id=self._next_id, username=username, email=email)
        self._users[self._next_id] = user
        self._next_id += 1
        return user

    def get(self, user_id: int) -> Optional[User]:
        return self._users.get(user_id)

    def find_by_email(self, email: str) -> Optional[User]:
        return next((u for u in self._users.values() if u.email == email), None)

    def all(self) -> list[User]:
        return list(self._users.values())


class TaskStore:
    """Simple in-memory task repository."""

    def __init__(self) -> None:
        self._tasks: dict[int, Task] = {}
        self._next_id = 1

    def create(
        self,
        title: str,
        description: str,
        assignee: Optional[User],
        priority: Priority,
    ) -> Task:
        task = Task(
            id=self._next_id,
            title=title,
            description=description,
            assignee=assignee,
            priority=priority,
        )
        self._tasks[self._next_id] = task
        self._next_id += 1
        return task

    def get(self, task_id: int) -> Optional[Task]:
        return self._tasks.get(task_id)

    def by_status(self, status: Status) -> list[Task]:
        return [t for t in self._tasks.values() if t.status == status]

    def by_assignee(self, user: User) -> list[Task]:
        return [t for t in self._tasks.values() if t.assignee and t.assignee.id == user.id]

    def overdue(self) -> list[Task]:
        return [t for t in self._tasks.values() if t.is_overdue()]


class ProjectStore:
    """Simple in-memory project repository."""

    def __init__(self, user_store: UserStore, task_store: TaskStore) -> None:
        self._projects: dict[int, Project] = {}
        self._user_store = user_store
        self._task_store = task_store
        self._next_id = 1

    def create(self, name: str, owner: User) -> Project:
        project = Project(id=self._next_id, name=name, owner=owner)
        self._projects[self._next_id] = project
        self._next_id += 1
        return project

    def get(self, project_id: int) -> Optional[Project]:
        return self._projects.get(project_id)

    def all(self) -> list[Project]:
        return list(self._projects.values())
