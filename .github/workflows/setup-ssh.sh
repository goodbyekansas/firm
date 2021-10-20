which ssh-agent || ( apt-get update -y && apt-get install openssh-client -y )
##
## Run ssh-agent (inside the build environment)
##
eval "$(ssh-agent -s)"
##
## Add the SSH key stored in SSH_PRIVATE_KEY variable to the agent store
## We're using tr to fix line endings which makes ed25519 keys work
## without extra base64 encoding.
## https://gitlab.com/gitlab-examples/ssh-private-key/issues/1#note_48526556
##
echo "$CI_ACCESS_KEY" | tr -d '\r' | ssh-add -
##

echo "SSH_AUTH_SOCK=$SSH_AUTH_SOCK" >> "$GITHUB_ENV"

# user ssh config
mkdir -p ~/.ssh
chmod 700 ~/.ssh
cp "$1" ~/.ssh/config

# root ssh config
sudo mkdir -p /root/.ssh
sudo chmod 700 /root/.ssh
sudo cp "$1" /root/.ssh/config
sudo chown root:root /root/.ssh/config

# root access key
echo "$CI_ACCESS_KEY" | sudo tee /root/.ssh/id_rsa >/dev/null
sudo chmod 600 /root/.ssh/id_rsa

# list public part of key for convenience
ssh-add -L
