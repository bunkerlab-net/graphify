-- #1910 regression: DDL embedded inside a PL/pgSQL body must not be recovered as
-- top-level objects, whether it appears mid-line (in a string literal) OR
-- line-leading inside the dollar-quoted body. Only the real CREATEs below are.
CREATE FUNCTION real_fn(n int) RETURNS int LANGUAGE plpgsql AS $$
BEGIN
CREATE FUNCTION body_leading_fake();
    PERFORM 'CREATE FUNCTION quoted_fake()';
    EXECUTE 'CREATE PROCEDURE proc_fake()';
    RETURN n;
END;
$$;

-- An `E'…'` escape string: the `\'` is an escaped quote, NOT a close, so the
-- line-leading CREATE inside the string must stay masked.
CREATE FUNCTION estr_fn() RETURNS text LANGUAGE sql AS E'prefix \'
CREATE FUNCTION estr_fake();
suffix';
