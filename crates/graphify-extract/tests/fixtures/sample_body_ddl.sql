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

-- A block comment (in the errored definition, before the `$$` body) whose body
-- has a line-leading CREATE: block-comment masking must blank it, not recover it.
CREATE FUNCTION blk_fn() RETURNS int LANGUAGE plpgsql AS /*
CREATE FUNCTION block_fake();
*/ $$
BEGIN RETURN 1; END;
$$;

-- An `E'…'` escape string: the `\'` is an escaped quote, NOT a close, so the
-- line-leading CREATE inside the string must stay masked.
CREATE FUNCTION estr_fn() RETURNS text LANGUAGE sql AS E'prefix \'
CREATE FUNCTION estr_fake();
suffix';
