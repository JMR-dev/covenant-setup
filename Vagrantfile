WINDOWS_BOX = ENV.fetch("COVENANT_WINDOWS_BOX", "gusztavvargadr/windows-11")
WINDOWS_BOX_VERSION = ENV["COVENANT_WINDOWS_BOX_VERSION"]
VM_NAME = ENV.fetch("COVENANT_VM_NAME", "covenant-setup-windows")
# Guest hostname must fit the 15-character NetBIOS limit, or Windows truncates
# it and Vagrant re-attempts the rename (with a reboot) on every up/reload.
VM_HOSTNAME = ENV.fetch("COVENANT_VM_HOSTNAME", "covenant-setup")
VM_MEMORY = ENV.fetch("COVENANT_VM_MEMORY", "6144")
VM_CPUS = ENV.fetch("COVENANT_VM_CPUS", "4")
WINRM_USERNAME = ENV.fetch("COVENANT_WINRM_USERNAME", "vagrant")
WINRM_PASSWORD = ENV.fetch("COVENANT_WINRM_PASSWORD", "vagrant")
HYPERV_SWITCH = ENV.fetch("COVENANT_HYPERV_SWITCH", "Default Switch")

Vagrant.configure("2") do |config|
  config.vm.box = WINDOWS_BOX
  if WINDOWS_BOX_VERSION && !WINDOWS_BOX_VERSION.empty?
    config.vm.box_version = WINDOWS_BOX_VERSION
  end

  config.vm.hostname = VM_HOSTNAME
  config.vm.guest = :windows
  config.vm.communicator = "winrm"
  config.vm.boot_timeout = ENV.fetch("COVENANT_BOOT_TIMEOUT", "1800").to_i
  config.vm.graceful_halt_timeout = 180
  config.vm.box_check_update = false
  config.vm.network "public_network", bridge: HYPERV_SWITCH
  config.vm.synced_folder ".", "/vagrant", disabled: true

  config.winrm.username = WINRM_USERNAME
  config.winrm.password = WINRM_PASSWORD
  config.winrm.retry_limit = 60
  config.winrm.retry_delay = 10
  config.winrm.timeout = 1800

  config.vm.provider "hyperv" do |h|
    h.vmname = VM_NAME
    h.memory = VM_MEMORY.to_i
    h.cpus = VM_CPUS.to_i
  end
end
