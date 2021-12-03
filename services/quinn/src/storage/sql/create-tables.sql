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

create table if not exists publishers (
    id uuid default uuid_generate_v4(),
    name varchar(128),
    email varchar(128),
    constraint name_email_key primary key(name, email),
    constraint publisher_id_unique unique(id)
);


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
    publisher_id uuid references publishers (id) not null,
    signature bytea null,
    constraint name_version_key primary key(name, version),
    constraint id_unique unique(id)
);


create table if not exists attachments (
    id uuid primary key default uuid_generate_v4(),
    name varchar(128),
    metadata hstore,
    checksums checksums,
    created_at timestamp default (now() at time zone 'utc'),
    publisher_id uuid references publishers (id),
    signature bytea null
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
        attachment_ids uuid[],
        publisher publishers
    );
exception
    when duplicate_object then null;
end $$;

do $$ begin
    create type attachment_with_publisher as (
        attachment attachments,
        publisher publishers
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
    attachment_ids uuid[],
    publisher_id uuid,
    signature bytea
) returns function_with_attachments as
$$
declare
    inserted_function functions;
begin

    insert into functions values (
        default, -- id -> generated
        name,
        version,
        metadata,
        code,
        required_inputs,
        optional_inputs,
        outputs,
        runtime,
        default, -- created at -> now
        publisher_id,
        signature
    ) returning * into inserted_function;

    -- insert attachment ids in relation table
    insert into attachments_to_functions values (inserted_function.id, unnest(attachment_ids));
    return row(inserted_function, attachment_ids, (select (publishers::publishers) from publishers where publishers.id = publisher_id limit 1))::function_with_attachments;
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
        array_remove(array_agg(attachments_to_functions.attachment_id), null),
        publishers::publishers
    )::function_with_attachments
    from functions
    left join attachments_to_functions on attachments_to_functions.function_id = functions.id
    left join publishers on publishers.id = functions.publisher_id
    where functions.name = name_ and functions.version = version_ group by functions.name, functions.version, publishers.* limit 1;
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


create or replace function list_functions_internal(
    name_ varchar(128),
    metadata_ hstore,
    offset_ bigint,
    limit_ bigint,
    order_by_ varchar(128),
    reverse_ bool,
    version_filters_ version_comparator[],
    publisher_email_ varchar(128),
    group_versions_ bool
) returns setof function_with_attachments as
$$
    with all_ as
    (
        select
            functions::functions as functions_,
            -- rust does not like nulls in the array (who does?)
            array_remove(array_agg(attachments_to_functions.attachment_id), null) as attachments_,
            publishers::publishers as publishers_,
            row_number() over (partition by functions.name order by version desc) as row_number_
        from functions
        left join attachments_to_functions on attachments_to_functions.function_id = functions.id
        left join publishers on publishers.id = functions.publisher_id
        where 
            case when group_versions_ then
                functions.name like ('%' || name_ || '%')
            else
                functions.name = name_
            end
        and
            version_matches(functions.version, version_filters_)
        group by functions.name, functions.version, publishers.*
    )
    select (functions_, attachments_, publishers_)::function_with_attachments from all_
    where
        (row_number_ = 1 or group_versions_ = false)
        and
            (
                (functions_).metadata ?& akeys(metadata_)
                and
                (
                    -- remove all null values since it is enough that they fulfill the above
                    coalesce((select hstore(array_agg(key), array_agg(value)) from each(metadata_) where value is not null), ''::hstore)
                ) <@ (functions_).metadata
            )
        and
            (publishers_).email like ('%' || publisher_email_ || '%')
    order by
        -- TODO: 🤮 This code is very ugly and there is most likely
        -- a better way to write this
        case when order_by_ = 'name_version' and not reverse_ then (functions_).name end asc,
        case when order_by_ = 'name_version' and reverse_ then (functions_).name end desc,
        case when order_by_ = 'name_version' and not reverse_ then (functions_).version end desc,
        case when order_by_ = 'name_version' and reverse_ then (functions_).version end asc
    offset offset_ limit limit_;
$$ language sql;

create or replace function list_functions(
    name_ varchar(128),
    metadata_ hstore,
    offset_ bigint,
    limit_ bigint,
    order_by_ varchar(128),
    reverse_ bool,
    version_filters_ version_comparator[],
    publisher_email_ varchar(128)
) returns setof function_with_attachments as
$$
    select list_functions_internal(
        name_,
        metadata_,
        offset_,
        limit_,
        order_by_,
        reverse_,
        version_filters_,
        publisher_email_,
        true
    );
$$ language sql;


create or replace function list_versions(
    name_ varchar(128),
    metadata_ hstore,
    offset_ bigint,
    limit_ bigint,
    order_by_ varchar(128),
    reverse_ bool,
    version_filters_ version_comparator[],
    publisher_email_ varchar(128)
) returns setof function_with_attachments as
$$
    select list_functions_internal(
        name_,
        metadata_,
        offset_,
        limit_,
        order_by_,
        reverse_,
        version_filters_,
        publisher_email_,
        false
    );
$$ language sql;


create or replace function get_attachment (
    id_ uuid
) returns setof attachment_with_publisher as
$$
    select (
        attachments::attachments,
        publishers::publishers
    )::attachment_with_publisher
    from attachments
    left join publishers on publishers.id = attachments.publisher_id
    where attachments.id = id_ group by attachments.id, publishers.* limit 1;
$$ language sql;


create or replace function insert_attachment (
    name varchar(128),
    metadata hstore,
    checksums checksums,
    publisher_id uuid,
    signature bytea
) returns attachment_with_publisher as
$$
declare
    inserted_attachment attachments;
begin
    insert into attachments values (
        default, -- id -> generated
        name,
        metadata,
        checksums,
        default, -- created_at -> now
        publisher_id,
        signature
    ) returning * into inserted_attachment;

    return row(inserted_attachment, (select (publishers::publishers) from publishers where publishers.id = publisher_id limit 1))::attachment_with_publisher;
end;
$$ language plpgsql;


create or replace function insert_or_get_publisher (
    _name varchar(128),
    _email varchar(128)
) returns publishers as
$$
declare
    inserted_publisher publishers;
begin
    insert into publishers values (
        default,
        _name,
        _email
    ) on conflict (name, email) do update set name = _name returning * into inserted_publisher;

    return inserted_publisher;
end;
$$ language plpgsql;
