include $(TOPDIR)/rules.mk

PKG_NAME:=kith
PKG_VERSION:=0.1.0
PKG_RELEASE:=1
PKG_LICENSE:=PolyForm-Noncommercial-1.0.0
PKG_LICENSE_FILES:=LICENSE.md
PKG_MAINTAINER:=kith developers

KITH_BIN_DIR?=$(TOPDIR)/dl/kith-$(PKG_VERSION)

include $(INCLUDE_DIR)/package.mk

ifeq ($(ARCH),x86_64)
  KITH_TRIPLE:=x86_64-unknown-linux-musl
else ifeq ($(ARCH),aarch64)
  KITH_TRIPLE:=aarch64-unknown-linux-musl
else ifeq ($(ARCH),arm)
  KITH_TRIPLE:=armv7-unknown-linux-musleabihf
else ifeq ($(ARCH),mipsel)
  KITH_TRIPLE:=mipsel-unknown-linux-musl
else ifeq ($(ARCH),mips)
  KITH_TRIPLE:=mips-unknown-linux-musl
else ifeq ($(ARCH),i386)
  KITH_TRIPLE:=i686-unknown-linux-musl
endif

define Package/kith
  SECTION:=net
  CATEGORY:=Network
  SUBMENU:=VPN
  TITLE:=kith WireGuard mesh VPN
  DEPENDS:=+kmod-tun @(x86_64||aarch64||arm||mipsel||mips||i386)
endef

define Package/kith/description
  kith is a WireGuard mesh VPN. This package installs a prebuilt static musl binary.
endef

define Package/kith/conffiles
/etc/kith/kith.toml
endef

define Build/Prepare
	mkdir -p $(PKG_BUILD_DIR)
	$(CP) $(KITH_BIN_DIR)/$(KITH_TRIPLE)/kith $(PKG_BUILD_DIR)/kith
endef

define Build/Configure
endef

define Build/Compile
endef

define Package/kith/install
	$(INSTALL_DIR) $(1)/usr/bin $(1)/etc/init.d $(1)/etc/kith
	$(INSTALL_BIN) $(PKG_BUILD_DIR)/kith $(1)/usr/bin/kith
	$(INSTALL_BIN) ./files/kith.init $(1)/etc/init.d/kith
	$(INSTALL_CONF) ./files/kith.conf $(1)/etc/kith/kith.toml
endef

$(eval $(call BuildPackage,kith))
