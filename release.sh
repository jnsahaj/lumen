#!/bin/bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOMEBREW_REPO="../homebrew-lumen"
FORMULA_PATH="$HOMEBREW_REPO/Formula/lumen.rb"

# Helper functions
info() { echo -e "${BLUE}[INFO]${NC} $1"; }
success() { echo -e "${GREEN}[SUCCESS]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

confirm() {
    local prompt="$1"
    local response
    echo -en "${YELLOW}$prompt [y/N]${NC} "
    read -r response
    [[ "$response" =~ ^[Yy]$ ]]
}

prompt_input() {
    local prompt="$1"
    local var_name="$2"
    local default="$3"
    local response
    
    if [[ -n "$default" ]]; then
        echo -en "${BLUE}$prompt${NC} [${default}]: "
    else
        echo -en "${BLUE}$prompt${NC}: "
    fi
    read -r response
    
    if [[ -z "$response" && -n "$default" ]]; then
        eval "$var_name='$default'"
    else
        eval "$var_name='$response'"
    fi
}

# Get current version from Cargo.toml
get_current_version() {
    grep '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/'
}

# Validate semantic version
validate_version() {
    local version="$1"
    if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        error "Invalid version format: $version. Expected format: X.Y.Z"
    fi
}

# Check prerequisites
check_prerequisites() {
    info "Checking prerequisites..."
    
    # Check if we're in a git repo
    if ! git rev-parse --is-inside-work-tree &>/dev/null; then
        error "Not in a git repository"
    fi
    
    # Check for uncommitted changes (excluding Cargo.toml and Cargo.lock)
    if [[ -n $(git status --porcelain | grep -v 'Cargo.toml' | grep -v 'Cargo.lock') ]]; then
        warn "You have uncommitted changes (other than Cargo.toml/Cargo.lock):"
        git status --short | grep -v 'Cargo.toml' | grep -v 'Cargo.lock'
        if ! confirm "Continue anyway?"; then
            exit 1
        fi
    fi
    
    # Check if cargo is installed
    if ! command -v cargo &>/dev/null; then
        error "cargo is not installed"
    fi
    
    # Check if gh CLI is installed
    if ! command -v gh &>/dev/null; then
        error "GitHub CLI (gh) is not installed. Install with: brew install gh"
    fi
    
    # Check if gh is authenticated
    if ! gh auth status &>/dev/null; then
        error "GitHub CLI is not authenticated. Run: gh auth login"
    fi
    
    # Check if homebrew repo exists
    if [[ ! -d "$HOMEBREW_REPO" ]]; then
        error "Homebrew repo not found at $HOMEBREW_REPO"
    fi
    
    # Check if Formula file exists
    if [[ ! -f "$FORMULA_PATH" ]]; then
        error "Formula file not found at $FORMULA_PATH"
    fi
    
    success "All prerequisites met"
}

# Step 1: Update version in Cargo.toml and Cargo.lock
update_cargo_version() {
    local new_version="$1"
    info "Updating Cargo.toml version to $new_version..."
    
    sed -i '' "s/^version = \".*\"/version = \"$new_version\"/" Cargo.toml
    
    # Verify the change
    local updated_version
    updated_version=$(get_current_version)
    if [[ "$updated_version" != "$new_version" ]]; then
        error "Failed to update Cargo.toml version"
    fi
    
    # Update Cargo.lock - need to update the package version directly
    info "Updating Cargo.lock..."
    # Find and replace the version for the lumen package in Cargo.lock
    # The format is: name = "lumen" followed by version = "X.Y.Z"
    sed -i '' '/^name = "lumen"$/{n;s/^version = ".*"/version = "'"$new_version"'"/;}' Cargo.lock
    
    # Run cargo check to ensure Cargo.lock is valid and update any dependency changes
    cargo check --quiet 2>/dev/null || cargo check
    
    success "Cargo.toml and Cargo.lock updated"
}

# Step 2: Commit version changes
commit_version_changes() {
    local version="$1"
    info "Committing version changes..."
    
    git add Cargo.toml Cargo.lock
    git commit -m "chore: bump version to $version"
    
    success "Version changes committed"
}

# Step 3: Publish to crates.io
publish_to_crates() {
    info "Publishing to crates.io..."
    
    if confirm "Run 'cargo publish --dry-run' first?"; then
        cargo publish --dry-run
        if ! confirm "Dry run successful. Proceed with actual publish?"; then
            error "Aborted by user"
        fi
    fi
    
    cargo publish
    
    success "Published to crates.io"
}

# Step 4: Build release binary
build_release() {
    info "Building release binary..."
    
    cargo build --release
    
    if [[ ! -f "target/release/lumen" ]]; then
        error "Release binary not found at target/release/lumen"
    fi
    
    success "Release binary built"
}

# Step 5: Create tarball
create_tarball() {
    info "Creating tarball..."
    
    cd target/release
    tar -czf lumen.tar.gz lumen
    cd "$SCRIPT_DIR"
    
    if [[ ! -f "target/release/lumen.tar.gz" ]]; then
        error "Failed to create tarball"
    fi
    
    success "Tarball created at target/release/lumen.tar.gz"
}

# Step 6: Calculate SHA256
calculate_sha256() {
    local sha256
    sha256=$(shasum -a 256 target/release/lumen.tar.gz | awk '{print $1}')
    echo "$sha256"
}

# Generate release notes from commits since last tag
generate_release_notes() {
    local version="$1"
    local last_tag
    local notes="## What's Changed\n\n"
    
    # Get the last tag (most recent tag before HEAD)
    last_tag=$(git describe --tags --abbrev=0 HEAD^ 2>/dev/null || echo "")
    
    if [[ -z "$last_tag" ]]; then
        # No previous tag, get all commits
        info "No previous tag found, including all commits"
        while IFS= read -r line; do
            local hash=$(echo "$line" | cut -d' ' -f1)
            local message=$(echo "$line" | cut -d' ' -f2-)
            notes+="* $message ([${hash:0:7}](https://github.com/jnsahaj/lumen/commit/$hash))\n"
        done < <(git log --oneline --format="%H %s")
    else
        info "Generating changelog since $last_tag"
        while IFS= read -r line; do
            local hash=$(echo "$line" | cut -d' ' -f1)
            local message=$(echo "$line" | cut -d' ' -f2-)
            notes+="* $message ([${hash:0:7}](https://github.com/jnsahaj/lumen/commit/$hash))\n"
        done < <(git log --oneline --format="%H %s" "$last_tag"..HEAD)
    fi
    
    echo -e "$notes"
}

# Step 7: Create GitHub release and upload
create_github_release() {
    local version="$1"
    local tag="v$version"
    
    info "Creating GitHub release $tag..."
    
    # Check if tag already exists
    if git tag -l | grep -q "^$tag$"; then
        warn "Tag $tag already exists locally"
        if ! confirm "Delete and recreate tag?"; then
            error "Aborted by user"
        fi
        git tag -d "$tag"
    fi
    
    # Create and push tag
    git tag "$tag"
    git push origin "$tag"
    
    # Generate release notes
    local release_notes
    release_notes=$(generate_release_notes "$version")
    
    # Create release with gh CLI
    gh release create "$tag" \
        --title "v$version" \
        --notes "$release_notes" \
        target/release/lumen.tar.gz
    
    success "GitHub release created and tarball uploaded"
}

# Step 8: Update homebrew formula
update_homebrew_formula() {
    local version="$1"
    local sha256="$2"
    local download_url="https://github.com/jnsahaj/lumen/releases/download/v$version/lumen.tar.gz"
    
    info "Updating homebrew formula..."
    
    cd "$HOMEBREW_REPO"
    
    # Pull latest changes
    git pull origin main --rebase
    
    # Update the formula
    sed -i '' "s|url \".*\"|url \"$download_url\"|" Formula/lumen.rb
    sed -i '' "s|sha256 \".*\"|sha256 \"$sha256\"|" Formula/lumen.rb
    sed -i '' "s|version \".*\"|version \"$version\"|" Formula/lumen.rb
    
    # Show the diff
    echo ""
    info "Changes to Formula/lumen.rb:"
    git diff Formula/lumen.rb
    echo ""
    
    if ! confirm "Commit and push these changes?"; then
        git checkout Formula/lumen.rb
        cd "$SCRIPT_DIR"
        error "Aborted by user"
    fi
    
    # Commit and push
    git add Formula/lumen.rb
    git commit -m "chore: Bump ver to $version"
    git push origin main
    
    cd "$SCRIPT_DIR"
    
    success "Homebrew formula updated and pushed"
}

# Push main branch changes
push_main_changes() {
    info "Pushing changes to main branch..."
    
    git push origin main
    
    success "Changes pushed to main"
}

# Main release flow
main() {
    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}       Lumen Release Script${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    
    # Change to script directory
    cd "$SCRIPT_DIR"
    
    # Check prerequisites
    check_prerequisites
    
    # Get current version
    local current_version
    current_version=$(get_current_version)
    info "Current version: $current_version"
    
    # Prompt for new version
    local new_version
    prompt_input "Enter new version" new_version ""
    
    if [[ -z "$new_version" ]]; then
        error "Version cannot be empty"
    fi
    
    validate_version "$new_version"
    
    if [[ "$new_version" == "$current_version" ]]; then
        error "New version is the same as current version"
    fi
    
    echo ""
    echo -e "${YELLOW}Release Plan:${NC}"
    echo "  1. Update Cargo.toml version to $new_version"
    echo "  2. Commit Cargo.toml and Cargo.lock"
    echo "  3. Publish to crates.io"
    echo "  4. Build release binary"
    echo "  5. Create tarball"
    echo "  6. Create GitHub release v$new_version and upload tarball"
    echo "  7. Push main branch changes"
    echo "  8. Update homebrew formula"
    echo ""
    
    if ! confirm "Proceed with release?"; then
        error "Aborted by user"
    fi
    
    echo ""
    
    # Execute release steps
    update_cargo_version "$new_version"
    echo ""
    
    commit_version_changes "$new_version"
    echo ""
    
    publish_to_crates
    echo ""
    
    build_release
    echo ""
    
    create_tarball
    echo ""
    
    local sha256
    sha256=$(calculate_sha256)
    info "SHA256: $sha256"
    echo ""
    
    create_github_release "$new_version"
    echo ""
    
    push_main_changes
    echo ""
    
    update_homebrew_formula "$new_version" "$sha256"
    echo ""
    
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}  Release v$new_version Complete!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo "Summary:"
    echo "  - Cargo.toml updated to v$new_version"
    echo "  - Published to crates.io"
    echo "  - GitHub release: https://github.com/jnsahaj/lumen/releases/tag/v$new_version"
    echo "  - Homebrew formula updated"
    echo ""
    echo "Users can now install with: brew install jnsahaj/lumen/lumen"
    echo ""
}

# Run main function
main "$@"
