"""Data models for the task management system."""
from __future__ import annotations
from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum
from typing import Optional


class Priority(Enum):
    LOW = 1
    MEDIUM = 2
    HIGH = 3
    CRITICAL = 4


class Status(Enum):
    TODO = "todo"
    IN_PROGRESS = "in_progress"
    DONE = "done"
    CANCELLED = "cancelled"


@dataclass
class User:
    """Represents a system user."""
    id: int
    username: str
    email: str
    created_at: datetime = field(default_factory=datetime.utcnow)

    def display_name(self) -> str:
        return self.username


@dataclass
class Task:
    """A unit of work to be completed."""
    id: int
    title: str
    description: str
    assignee: Optional[User]
    priority: Priority
    status: Status = Status.TODO
    created_at: datetime = field(default_factory=datetime.utcnow)
    due_date: Optional[datetime] = None
    tags: list[str] = field(default_factory=list)

    def is_overdue(self) -> bool:
        if self.due_date is None:
            return False
        return datetime.utcnow() > self.due_date and self.status != Status.DONE

    def assign_to(self, user: User) -> None:
        self.assignee = user
        self.status = Status.IN_PROGRESS

    def complete(self) -> None:
        self.status = Status.DONE


@dataclass
class Project:
    """A container for related tasks."""
    id: int
    name: str
    owner: User
    tasks: list[Task] = field(default_factory=list)

    def add_task(self, task: Task) -> None:
        self.tasks.append(task)

    def open_tasks(self) -> list[Task]:
        return [t for t in self.tasks if t.status != Status.DONE]

    def completion_ratio(self) -> float:
        if not self.tasks:
            return 0.0
        done = sum(1 for t in self.tasks if t.status == Status.DONE)
        return done / len(self.tasks)
