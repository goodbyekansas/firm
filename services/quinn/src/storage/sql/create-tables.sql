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


do $$ begin
    create type version as (
        major integer,
        minor integer,
        patch integer,
        pre   varchar(256),
        build varchar(256)
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
    version version,
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
    version version,
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

do $$ begin
    create type function_with_attachments as (
        func functions,
        attachment_ids uuid[]
    );
exception
    when duplicate_object then null;
end $$;

create or replace function get_function (
    id_ uuid
) returns setof function_with_attachments as
$$
    select (
        functions::functions,
        -- rust does not like nulls in the array (who does?)
        array_remove(array_agg(attachments_to_functions.attachment_id), null)
    )::function_with_attachments
    from functions
    left join attachments_to_functions on attachments_to_functions.function_id = functions.id
    where functions.id = id_ group by functions.id limit 1;
$$ language sql;

do $$ begin
    create type version_comparator as (
        version version,
        op varchar(2)
    );
exception
    when duplicate_object then null;
end $$;

create or replace function version_matches(
    version_ version,
    comparator version_comparator
) returns boolean as
$$
    select case
        when (comparator).op is null then true
        when coalesce(version_.pre, '') != coalesce((comparator).version.pre, '') then false
        when (comparator).op = '='  or (comparator).version.pre is not null then version_ = (comparator).version
        when (comparator).op = '<'  then version_ < (comparator).version
        when (comparator).op = '<=' then version_ <= (comparator).version
        when (comparator).op = '>'  then version_ > (comparator).version
        when (comparator).op = '>=' then version_ >= (comparator).version
    end;
$$ language sql;


create or replace function version_matches(
    version_ version,
    comparators version_comparator[]
) returns boolean as
$$
    select bool_and(result.match) from (
        select version_matches(version_, comparator) as match
            from unnest(comparators) as comparator
        union
        select true as match
    ) as result;
$$ language sql;


create or replace function list_functions (
    name_ varchar(128),
    exact_name_match bool,
    metadata_ hstore,
    offset_ bigint,
    limit_ bigint,
    order_by_ varchar(128),
    order_descending_ bool,
    version_filters version_comparator[]
) returns setof function_with_attachments as
$$
    select (
        functions::functions,
        -- rust does not like nulls in the array (who does?)
        array_remove(array_agg(attachments_to_functions.attachment_id), null)
    )::function_with_attachments
    from functions
    left join attachments_to_functions on attachments_to_functions.function_id = functions.id
    where 
        case when exact_name_match = false then
            functions.name like ('%' || name_ || '%')
        else
            functions.name = name_
        end
    and
        (
            metadata ?& akeys(metadata_)
            and
            (
            -- remove all null values since it is enough that they fulfill the above
            coalesce((select hstore(array_agg(key), array_agg(value)) from each(metadata_) where value is not null), ''::hstore)
            ) <@ metadata
        )
    and
        version_matches(functions.version, version_filters)
    group by functions.id
    order by
        case when order_by_ = 'name' and not order_descending_ then functions.name end asc,
        case when order_by_ = 'name' and order_descending_ then functions.name end desc,
        functions.version desc
    offset offset_ limit limit_;
$$ language sql;


do $$ begin
    create type attachment_with_functions as (
        attachment attachments,
        function_ids uuid[]
    );
exception
    when duplicate_object then null;
end $$;

create or replace function get_attachment (
    id_ uuid
) returns setof attachment_with_functions as
$$
    select (
        attachments::attachments,
        array_remove(array_agg(attachments_to_functions.function_id), null)
    )::attachment_with_functions
    from attachments
    left join attachments_to_functions on (attachments.id = attachments_to_functions.attachment_id)
    where attachments.id = id_ group by attachments.id limit 1;
$$ language sql;


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
