get_dirs() {
    server_name="$1"

    mkdir -p .postgres-servers
    datadir_file=.postgres-servers/$server_name.datadir
    socket_dir_file=.postgres-servers/$server_name.sockdir
}

start_postgres_server() {
    server_name="$1"

    if [ -z $server_name ]; then
        echo "Please give your server a name"
        return 1
    fi

    datadir=$(mktemp -d)
    socket_dir=$(mktemp -d)

    get_dirs $server_name

    if [ -f datadir_file ]; then
        echo "You already have a server running with name $server_name."
        echo "Stop it with stop_postgres_server $server_name"
    fi

    pg_ctl -D $datadir initdb | sed "s/^/  🐘 [postgres] /" || return 1
    pg_ctl start -D $datadir -o "-k $socket_dir -c listen_addresses=''" || return 1
    createdb -h $socket_dir functions || return 1
    echo $datadir >$datadir_file
    echo $socket_dir >$socket_dir_file

    echo "🎆 Server started! Use this connection string: postgres:///functions?host=$socket_dir&user=$(id -un)"
    echo "To stop it, run stop_postgres_server $server_name"
    echo "To connect to to it run psql_to_server $server_name"
}

list_postgres_servers() (
    shopt -s nullglob
    for f in ./.postgres-servers/*.datadir; do
        echo $(basename ${f/.datadir//})
    done
)

stop_all_postgres_servers() {
    for server in $(list_postgres_servers); do
        stop_postgres_server $server
    done
}

stop_postgres_server() {
    server_name="$1"

    echo "Stopping server $server_name..."

    get_dirs $server_name
    datadir=$(cat $datadir_file)

    pg_ctl stop -D $datadir | sed "s/^/  🐘 [postgres] /"
    rm $datadir_file
    rm $socket_dir_file

    echo "Server $server_name stopped!"
}

psql_to_server() {
    server_name="$1"
    shift

    get_dirs $server_name
    psql -h $(cat $socket_dir_file) functions "$@"
}

postgres_tests() {
    id=$(dd bs=18 count=1 if=/dev/urandom status=none | base64 | tr +/ _.)
    name="cargo-test-$id"

    start_postgres_server $name
    get_dirs $name

    uri="postgres:///functions?host=$(cat $socket_dir_file)&user=$(id -un)"
    REGISTRY_functions_storage_uri=$uri cargo test --features="postgres-tests" --lib
    stop_postgres_server $name
}

# TODO add trap to stop servers when exiting shell
