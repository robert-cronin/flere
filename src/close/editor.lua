local dir = debug.getinfo(1, 'S').source:sub(2):match('^(.*)/[^/]+$')
local last
local last_jump
local function write(name, text)
  vim.fn.writefile({text}, dir .. '/' .. name)
  vim.fn.setfperm(dir .. '/' .. name, 'rw-------')
end
local function state()
  local dirty, reason, processes, services = 0, '', {}, {}
  for _, b in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_valid(b) then
      if vim.bo[b].modified then dirty = dirty + 1 end
      local kind = vim.bo[b].buftype
      if kind == 'terminal' then
        if vim.fn.jobwait({vim.bo[b].channel}, 0)[1] == -1 then
          reason = 'An editor terminal is still running'
        end
      elseif kind ~= '' and kind ~= 'help' and kind ~= 'quickfix' then
        reason = 'The editor has a special buffer with unknown state'
      end
    end
  end
  if dirty > 0 then reason = string.format('%d unsaved buffer%s', dirty, dirty == 1 and '' or 's') end
  local mode = vim.api.nvim_get_mode()
  if mode.blocking or (mode.mode ~= 'n' and mode.mode ~= 'i') or vim.fn.getchar(1) ~= 0 then
    reason = 'The editor is busy or has a pending command'
  end
  -- The shared event loop also has non-Lua handles; luv.walk can abort on them.
  -- Include every child, including jobs started outside Lua. Flere separately
  -- verifies this list and matches each process against registered LSP commands.
  processes = vim.api.nvim_get_proc_children(vim.fn.getpid())
  table.sort(processes)
  local get_clients = package.loaded['vim.lsp']
    and (vim.lsp.get_clients or vim.lsp.get_active_clients)
  if get_clients then
    for _, client in ipairs(get_clients()) do
      local cmd = client.config.cmd
      if not client:is_stopped() and type(cmd) == 'table' and #cmd > 0 then
        services[#services + 1] = cmd
      end
    end
  end
  return reason, processes, services
end
write('ready', tostring(vim.fn.getpid()))
write('jump-ready', tostring(vim.fn.getpid()))
vim.fn.timer_start(60, function()
  pcall(function()
    if vim.fn.filereadable(dir .. '/jump.json') ~= 1 then return end
    local request = vim.fn.json_decode(table.concat(vim.fn.readfile(dir .. '/jump.json', '', 2), ''))
    if request.pid ~= vim.fn.getpid() or request.expires < os.time() or request.token == last_jump then return end
    last_jump = request.token
    if type(request.line) ~= 'number' or type(request.column) ~= 'number'
        or request.line < 1 or request.line > 100000000 or request.column < 1 or request.column > 1000000 then return end
    local mode = vim.api.nvim_get_mode()
    if mode.blocking or mode.mode ~= 'n' or vim.fn.getchar(1) ~= 0
        or vim.fn.resolve(vim.fn.expand('%:p')) ~= request.path then
      vim.api.nvim_echo({{'Flere: jump skipped; editor is busy or showing another file'}}, false, {})
      return
    end
    vim.fn.cursor(request.line, request.column)
    vim.cmd('normal! zz')
  end)
  pcall(function()
    if vim.fn.filereadable(dir .. '/request') ~= 1 then return end
    local request = vim.fn.readfile(dir .. '/request', '', 5)
    if #request < 4 or #request[1] ~= 32 or request[1]:find('[^0-9a-f]')
        or not tonumber(request[3]) or tonumber(request[3]) < os.time()
        or (request[2] ~= 'probe' and request[2] ~= 'commit') then return end
    local key = request[1] .. request[2]
    if last == key then return end
    last = key
    local reason, processes, services = state()
    if request[2] == 'commit' and table.concat(processes, ',') ~= request[4] then
      reason = 'Editor jobs changed while closing'
    end
    local reply = {token=request[1], phase=request[2], pid=vim.fn.getpid(),
      reason=reason, processes=processes, services=services}
    write('reply', vim.fn.json_encode(reply))
    if request[2] == 'commit' and reason == '' and vim.fn.filereadable(dir .. '/request') == 1 then
      local aw, awa, confirm = vim.o.autowrite, vim.o.autowriteall, vim.o.confirm
      vim.o.autowrite, vim.o.autowriteall, vim.o.confirm = false, false, false
      local ok = pcall(vim.cmd, 'qall')
      vim.o.autowrite, vim.o.autowriteall, vim.o.confirm = aw, awa, confirm
      if not ok then
        reply.reason = 'The editor refused to quit'
        write('reply', vim.fn.json_encode(reply))
      end
    end
  end)
end, {['repeat']=-1})
