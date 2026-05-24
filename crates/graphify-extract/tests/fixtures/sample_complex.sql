-- Schema-qualified tables
CREATE SCHEMA myapp;

CREATE TABLE myapp.organizations (
  id SERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE myapp.users (
  id SERIAL PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  org_id INT NOT NULL,
  manager_id INT,
  CONSTRAINT fk_users_org FOREIGN KEY (org_id) REFERENCES myapp.organizations(id),
  CONSTRAINT fk_users_mgr FOREIGN KEY (manager_id) REFERENCES myapp.users(id)
);

CREATE TABLE myapp.tasks (
  id SERIAL PRIMARY KEY,
  assignee_id INT REFERENCES myapp.users(id),
  org_id INT REFERENCES myapp.organizations(id),
  status TEXT
);

-- Index
CREATE INDEX idx_tasks_assignee ON myapp.tasks(assignee_id);
CREATE INDEX idx_users_email ON myapp.users(email);

-- View joining several tables
CREATE VIEW myapp.active_user_tasks AS
  SELECT u.email, t.status, o.name AS org_name
  FROM myapp.tasks t
  JOIN myapp.users u ON u.id = t.assignee_id
  JOIN myapp.organizations o ON o.id = t.org_id
  WHERE t.status = 'open';

-- Materialised view
CREATE MATERIALIZED VIEW myapp.task_summary AS
  SELECT org_id, COUNT(*) AS count FROM myapp.tasks GROUP BY org_id;

-- Functions and procedures
CREATE FUNCTION myapp.get_user_count(my_org_id INT) RETURNS BIGINT AS $$
  BEGIN
    RETURN (SELECT COUNT(*) FROM myapp.users WHERE org_id = my_org_id);
  END;
$$ LANGUAGE plpgsql;

CREATE PROCEDURE myapp.archive_user(uid INT) AS $$
  BEGIN
    UPDATE myapp.users SET email = 'archived' WHERE id = uid;
  END;
$$ LANGUAGE plpgsql;

-- Trigger
CREATE TRIGGER trg_users_updated
  BEFORE UPDATE ON myapp.users
  FOR EACH ROW EXECUTE FUNCTION myapp.touch_updated_at();

-- Alter table to add constraint
ALTER TABLE myapp.tasks
  ADD CONSTRAINT fk_tasks_org FOREIGN KEY (org_id) REFERENCES myapp.organizations(id);

-- Sequence
CREATE SEQUENCE myapp.user_seq START 100;
