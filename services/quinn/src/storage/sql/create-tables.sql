create extension if not exists "uuid-ossp";
create extension if not exists "hstore";


---------------------------------------------
-- Composite Types
---------------------------------------------

do $$ begin
    create type runtime as (
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
    create type channel_spec as (
        name varchar(128),
        description text,
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
    id uuid default uuid_generate_v4(),
    name varchar(128),
    version version,
    metadata hstore,
    code uuid null,
    required_inputs channel_spec[],
    optional_inputs channel_spec[],
    outputs channel_spec[],
    runtime runtime,
    created_at timestamp default (now() at time zone 'utc'),
    constraint name_version_key primary key(name, version),
    constraint id_unique unique(id)
);


create table if not exists attachments (
    id uuid primary key default uuid_generate_v4(),
    name varchar(128),
    metadata hstore,
    checksums checksums,
    created_at timestamp default (now() at time zone 'utc')
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

do $$ begin
    create type function_with_attachments as (
        func functions,
        attachment_ids uuid[]
    );
exception
    when duplicate_object then null;
end $$;

create or replace function insert_function (
    name varchar(128),
    version version,
    metadata hstore,
    code uuid,
    required_inputs channel_spec[],
    optional_inputs channel_spec[],
    outputs channel_spec[],
    runtime runtime,
    attachment_ids uuid[]
) returns function_with_attachments as
$$
declare
    inserted_function functions;
begin

    insert into functions values (
        default,
        name,
        version,
        metadata,
        code,
        required_inputs,
        optional_inputs,
        outputs,
        runtime,
        default
    ) returning * into inserted_function;

    -- insert attachment ids in relation table
    insert into attachments_to_functions values (inserted_function.id, unnest(attachment_ids));

    return row(inserted_function, attachment_ids)::function_with_attachments;
end;
$$ language plpgsql;

create or replace function get_function (
    name_ varchar(128),
    version_ version
) returns setof function_with_attachments as
$$
    select (
        functions::functions,
        -- rust does not like nulls in the array (who does?)
        array_remove(array_agg(attachments_to_functions.attachment_id), null)
    )::function_with_attachments
    from functions
    left join attachments_to_functions on attachments_to_functions.function_id = functions.id
    where functions.name = name_ and functions.version = version_ group by functions.name, functions.version limit 1;
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
    reverse_ bool,
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
    group by functions.name, functions.version
    order by
        -- TODO: 🤮 This code is very ugly and there is most likely
        -- a better way to write this
        case when order_by_ = 'name_version' and not reverse_ then functions.name end asc,
        case when order_by_ = 'name_version' and reverse_ then functions.name end desc,
        case when order_by_ = 'name_version' and not reverse_ then functions.version end desc,
        case when order_by_ = 'name_version' and reverse_ then functions.version end asc
    offset offset_ limit limit_;
$$ language sql;


create or replace function get_attachment (
    id_ uuid
) returns setof attachments as
$$
    select *
    from attachments
    where attachments.id = id_ group by attachments.id limit 1;
$$ language sql;


create or replace function insert_attachment (
    name varchar(128),
    metadata hstore,
    checksums checksums
) returns attachments as
$$
declare
    inserted_attachment attachments;
begin
    insert into attachments values (
        default,
        name,
        metadata,
        checksums,
        default
    ) returning * into inserted_attachment;

    return inserted_attachment;
end;
$$ language plpgsql;
