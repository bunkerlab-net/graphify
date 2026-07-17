-- #1910 regression: DDL embedded inside a PL/pgSQL body (string literals) must
-- not be recovered as top-level objects. Only the line-leading CREATE below is.
CREATE FUNCTION real_fn(n int) RETURNS int LANGUAGE plpgsql AS $$
BEGIN
    PERFORM 'CREATE FUNCTION fake_fn()';
    EXECUTE 'CREATE PROCEDURE fake_proc()';
    RETURN n;
END;
$$;
