create extension if not exists "uuid-ossp";
create extension if not exists "hstore";


---------------------------------------------
-- Composite Types
---------------------------------------------

do $$ begin
    create type execution_environment as (
        name varchar(128),
        entrypoint varchar(128),
        arguments hstore
    );
exception
    when duplicate_object then null;
end $$;

do $$ begin
    create type argument_type as enum ('string', 'float', 'bool', 'int', 'bytes');
exception
    when duplicate_object then null;
end $$;

do $$ begin
    create type function_input as (
        name varchar(128),
        required bool,
        argument_type argument_type,
        default_value varchar(128),
        from_execution_environment bool
    );
exception
    when duplicate_object then null;
end $$;

do $$ begin
    create type function_output as (
        name varchar(128),
        argument_type argument_type
    );
exception
    when duplicate_object then null;
end $$;

do $$ begin
    create type checksums as (
        sha256 char(64)
    );
exception
    when duplicate_object then null;
end $$;


------------------------------------------
-- Tables
------------------------------------------

create table if not exists functions (
    id uuid primary key default uuid_generate_v4(),
    name varchar(128),
    version varchar(128),
    metadata hstore,
    code uuid null,
    inputs function_input[],
    outputs function_output[],
    execution_environment execution_environment,
    constraint name_version_unique unique(name, version)
);


create table if not exists attachments (
    id uuid primary key default uuid_generate_v4(),
    name varchar(128),
    metadata hstore,
    checksums checksums
);

create table if not exists attachments_to_functions (
    function_id uuid references functions (id) on delete cascade on update cascade,
    attachment_id uuid references attachments (id) on delete cascade on update cascade,
    constraint function_attachment primary key(function_id, attachment_id)
);


------------------------------------------------
-- Functions
------------------------------------------------

create or replace function clear_tables() returns void as
$$
begin
    truncate table functions cascade;
    truncate table attachments cascade;
end;
$$ language plpgsql;

create or replace function insert_function (
    name varchar(128),
    version varchar(128),
    metadata hstore,
    code uuid,
    inputs function_input[],
    outputs function_output[],
    execution_environment execution_environment,
    attachment_ids uuid[]
) returns uuid as
$$
declare
    generated_id uuid;
begin

    insert into functions values (
        default,
        name,
        version,
        metadata,
        code,
        inputs,
        outputs,
        execution_environment
    ) returning id into generated_id;

    -- insert attachment ids in relation table
    insert into attachments_to_functions values (generated_id, unnest(attachment_ids));

    return generated_id;
end;
$$ language plpgsql;

create or replace function insert_attachment (
    name varchar(128),
    metadata hstore,
    checksums checksums
) returns uuid as
$$
declare
    generated_id uuid;
begin
    insert into attachments values (
        default,
        name,
        metadata,
        checksums
    ) returning id into generated_id;

    return generated_id;
end;
$$ language plpgsql;
