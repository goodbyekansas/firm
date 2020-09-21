which ssh-agent || ( apt-get update -y && apt-get install openssh-client -y )
##
## Run ssh-agent (inside the build environment)
##
eval $(ssh-agent -s)
##
## Add the SSH key stored in SSH_PRIVATE_KEY variable to the agent store
## We're using tr to fix line endings which makes ed25519 keys work
## without extra base64 encoding.
## https://gitlab.com/gitlab-examples/ssh-private-key/issues/1#note_48526556
##
echo "$CI_ACCESS_KEY" | tr -d '\r' | ssh-add -
##
echo "::set-env name=SSH_AUTH_SOCK::$SSH_AUTH_SOCK"

# set up for root
sudo mkdir -p /root/.ssh
sudo chmod 700 /root/.ssh
echo "$CI_ACCESS_KEY" | sudo tee /root/.ssh/id_rsa >/dev/null
sudo chmod 600 /root/.ssh/id_rsa

ssh-add -L