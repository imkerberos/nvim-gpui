Name:           nvim-gpui
Version:        0.7.2
Release:        1%{?dist}
Summary:        Another GPU-rendered graphical client for Neovim

License:        MIT
URL:            https://github.com/imkerberos/nvim-gpui
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  bsdtar
BuildRequires:  cmake
BuildRequires:  desktop-file-utils
BuildRequires:  fontconfig-devel
BuildRequires:  freetype-devel
BuildRequires:  libX11-devel
BuildRequires:  libxcb-devel
BuildRequires:  libXcursor-devel
BuildRequires:  libXi-devel
BuildRequires:  libXrandr-devel
BuildRequires:  libxkbcommon-devel
BuildRequires:  libxkbcommon-x11-devel
BuildRequires:  mesa-libEGL-devel
BuildRequires:  mesa-libGL-devel
BuildRequires:  neovim
BuildRequires:  openssl-devel
BuildRequires:  pkgconf-pkg-config
BuildRequires:  wayland-devel

Requires:       fontconfig
Requires:       freetype
Requires:       libX11
Requires:       libxcb
Requires:       libXcursor
Requires:       libXi
Requires:       libXrandr
Requires:       libglvnd-egl
Requires:       libglvnd-glx
Requires:       libglvnd-opengl
Requires:       libxkbcommon
Requires:       libxkbcommon-x11
Requires:       libwayland-client
Recommends:     brise librime
Recommends:     neovim >= 0.10.0

%description
nvim-gpui is another GPU-rendered Neovim client for a native,
cross-platform desktop experience. It supports local embedded sessions and
connections to remote Neovim instances through the Neovim msgpack-RPC UI
protocol.

%prep
rm -rf %{_builddir}/%{name}-%{version}
mkdir -p %{_builddir}
bsdtar -xf %{SOURCE0} -C %{_builddir}

%build
cd %{_builddir}/%{name}-%{version}
export CARGO_NET_OFFLINE=true
cargo build --release --locked --bins --jobs 2

%check
cd %{_builddir}/%{name}-%{version}
desktop-file-validate packaging/debian/nvim-gpui.desktop
cargo test --release --locked --offline --all-targets --jobs 2

%install
cd %{_builddir}/%{name}-%{version}
install -D -m 0755 target/release/nvim-gpui \
    %{buildroot}%{_bindir}/nvim-gpui
install -D -m 0755 target/release/gpvim \
    %{buildroot}%{_bindir}/gpvim
ln -s gpvim %{buildroot}%{_bindir}/gpvimdiff

install -D -m 0644 packaging/debian/nvim-gpui.desktop \
    %{buildroot}%{_datadir}/applications/nvim-gpui.desktop
for icon in packaging/debian/icons/hicolor/*/apps/nvim-gpui.png; do
    install -D -m 0644 "$icon" \
        %{buildroot}%{_datadir}/icons/hicolor/"${icon#packaging/debian/icons/hicolor/}"
done

install -D -m 0644 README.md \
    %{buildroot}%{_docdir}/nvim-gpui/README.md
install -D -m 0644 CHANGELOG.md \
    %{buildroot}%{_docdir}/nvim-gpui/CHANGELOG.md
install -D -m 0644 LICENSE \
    %{buildroot}%{_licensedir}/nvim-gpui/LICENSE

%files
%{_licensedir}/nvim-gpui/LICENSE
%{_docdir}/nvim-gpui/README.md
%{_docdir}/nvim-gpui/CHANGELOG.md
%{_bindir}/nvim-gpui
%{_bindir}/gpvim
%{_bindir}/gpvimdiff
%{_datadir}/applications/nvim-gpui.desktop
%{_datadir}/icons/hicolor/*/apps/nvim-gpui.png

%changelog
* Fri Sep 11 2026 nvim-gpui contributors - 0.7.2-1
- Improve Rime context isolation and asynchronous initial deployment.

* Thu Sep 10 2026 nvim-gpui contributors - 0.7.1-1
- Add the initial Fedora RPM packaging.
