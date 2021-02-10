#! /usr/bin/env nix-shell
#! nix-shell -i bash -p jq curl

shopt -s nullglob

get_package_name() {
  package_name=$(nix show-derivation "$1" | jq -r '.[] | .env | .pname // .name')
}

github_pre_build_hook() {
  repo="$1"
  sha="$2"
  run_id="$3"
  derivations_dir="$4"
  index=0
  mkdir -p .check_ids

  for drv in $derivations_dir/*; do
    get_package_name $drv
    echo "Posting build start for $package_name to github."

    payload="{
          \"name\": \"Build $package_name\",
          \"head_sha\": \"$sha\",
          \"status\": \"in_progress\",
          \"details_url\": \"https://github.com/$repo/runs/$run_id\"
        }
        "

    if [[ ! -z $GITHUB_TOKEN ]]; then
      curl_result=$(curl -s -X POST \
        https://api.github.com/repos/${repo}/check-runs \
        -H "Accept: application/vnd.github.antiope-preview+json" \
        -H 'Content-Type: text/json; charset=utf-8' \
        -H "Authorization: Bearer $GITHUB_TOKEN" \
        -d "$payload")

      echo $(jq -r ".id" <<<"$curl_result") >.check_ids/$index
    else
      echo "GITHUB_TOKEN not set, this would have been the payload:"
      echo "$payload"
      echo "0" >.check_ids/$index
    fi
    ((++index)) # This is how you increment stuff apparently.
  done
}

all_output_paths_exists() {
  # If the path exists nix has succeeded realising the derivation
  out_paths=$(nix show-derivation "$1" | jq -r '.[] | .outputs[].path')
  output_paths_exists=true
  for out_path in $out_paths; do
    if [[ ! -d $out_path ]]; then
      output_paths_exists=false
      return
    fi
  done
}

get_log_output() {
  log_output=$(nix-store --read-log "$1" 2>/dev/null)
  if [ ! $? -eq 0 ]; then
    log_output=""
  fi
}

github_post_build_hook() {
  repo="$1"
  derivations_dir="$2"
  index=0

  for drv in $derivations_dir/*; do
    get_package_name $drv
    check_id=$(cat .check_ids/$index)
    get_log_output $drv
    all_output_paths_exists $drv
    [[ ! -z $log_output ]] && log_exists=true || log_exists=false
    case $output_paths_exists$log_exists in
    truetrue)
      conclusion="success"
      ;;
    truefalse)
      conclusion="success"
      log_output="$package_name was retrieved from cache, skipping build"
      ;;
    falsefalse)
      conclusion="cancelled"
      log_output="Build was cancelled due to other package failure, see full log"
      ;;
    *)
      conclusion="failure"
      ;;
    esac

    echo "Posting build result ($conclusion) for $package_name"

    # log output can contain any random chars and needs to be escaped
    # here, we generate a jq expression to use later, hence the `\$log_output`
    log_output="\`\`\`
$log_output
\`\`\`"

    # save to file since it can be big
    echo "$log_output" > log.txt

    payload="{
      \"status\": \"completed\",
      \"conclusion\": \"$conclusion\",
      \"output\": {
        \"title\": \"Build $package_name\",
        \"summary\": \"Output from running nix build with checks on $package_name\",
        \"text\": \$log_output
      }
    }"

    if [[ ! -z $GITHUB_TOKEN ]]; then
      jq --null-input --rawfile log_output log.txt --compact-output "$payload" | curl -s -X PATCH \
        https://api.github.com/repos/$repo/check-runs/$check_id \
        -H "Accept: application/vnd.github.antiope-preview+json" \
        -H 'Content-Type: text/json; charset=utf-8' \
        -H "Authorization: Bearer $GITHUB_TOKEN" \
        -d @- > /dev/null
    else
      echo "GITHUB_TOKEN not set, this would have been the payload:"
      echo "$payload"
    fi
    ((++index)) # This is how you increment stuff apparently.
  done
}

command="$1"
case $command in
"pre")
  shift
  github_pre_build_hook $@
  ;;

"post")
  shift
  github_post_build_hook $@
  ;;
*)
  echo "Unknown command $command."
  exit 1
  ;;
esac
