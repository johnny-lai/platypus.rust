BUILD_ROOT ?= tmp
MEMCACHED_VERSION ?= memcached-1.6.39
MEMCACHED_TAR_GZ ?= $(MEMCACHED_VERSION).tar.gz
MEMCACHED_CONFIGURE_FLAGS ?= --with-libevent=$(shell brew --prefix libevent)

$(BUILD_ROOT)/memcached: $(BUILD_ROOT)/$(MEMCACHED_VERSION)/memcached
	ln -sf $(MEMCACHED_VERSION)/memcached memcached

clean:
	rm -rf $(BUILD_ROOT)

# Install from https://docs.memcached.org/features/proxy/quickstart/
$(BUILD_ROOT)/$(MEMCACHED_VERSION)/memcached: $(BUILD_ROOT)/$(MEMCACHED_VERSION)/README.md
	cd $(BUILD_ROOT)/$(MEMCACHED_VERSION) && ./configure --enable-proxy $(MEMCACHED_CONFIGURE_FLAGS) && make
	touch $(BUILD_ROOT)/$(MEMCACHED_VERSION)/memcached

$(BUILD_ROOT)/$(MEMCACHED_TAR_GZ):
	mkdir -p $(BUILD_ROOT)
	curl https://www.memcached.org/files/$(MEMCACHED_TAR_GZ) -o $@

$(BUILD_ROOT)/$(MEMCACHED_VERSION)/README.md: $(BUILD_ROOT)/$(MEMCACHED_TAR_GZ)
	tar zxf $< -C $(BUILD_ROOT)/
	touch $@
