" Private per-process integration. Timers never inject terminal keystrokes.
if !exists('*timer_start') || !exists('*json_encode')
  finish
endif
let s:dir = expand('<sfile>:p:h')
if has('nvim')
  execute 'lua dofile(' . json_encode(s:dir . '/editor.lua') . ')'
  finish
endif
function! s:reply(value) abort
  call writefile([json_encode(a:value)], s:dir . '/reply')
  call setfperm(s:dir . '/reply', 'rw-------')
endfunction
function! s:jump() abort
  if !filereadable(s:dir . '/jump.json') | return | endif
  try
    let request = json_decode(join(readfile(s:dir . '/jump.json', '', 2), ''))
    if get(request, 'pid', 0) != getpid() || get(request, 'expires', 0) < localtime() || get(request, 'token', '') ==# get(s:, 'last_jump', '') | return | endif
    let s:last_jump = request.token
    if request.line < 1 || request.column < 1 || request.line > 100000000 || request.column > 1000000 | return | endif
    if mode(1) !=# 'n' || getchar(1) != 0 || resolve(expand('%:p')) !=# request.path
      echo 'Flere: jump skipped; editor is busy or showing another file'
      return
    endif
    call cursor(request.line, request.column)
    normal! zz
  catch
    " Malformed or stale data cannot change the editor's buffer or input.
  endtry
endfunction
function! s:tick(timer) abort
  call s:jump()
  if !filereadable(s:dir . '/request') | return | endif
  try
    let request = readfile(s:dir . '/request', '', 5)
    if len(request) < 4 || request[0] !~# '^\x\{32}$' || str2nr(request[2]) < localtime() || index(['probe', 'commit'], request[1]) < 0
      return
    endif
    let key = request[0] . request[1]
    if get(s:, 'last', '') ==# key | return | endif
    let s:last = key
    let reason = ''
    let dirty = len(filter(getbufinfo(), 'v:val.changed'))
    if dirty
      let reason = printf('%d unsaved buffer%s', dirty, dirty == 1 ? '' : 's')
    elseif index(['n', 'i'], mode(1)) < 0 || getchar(1) != 0
      let reason = 'The editor is busy or has a pending command'
    else
      for buffer in getbufinfo()
        let kind = getbufvar(buffer.bufnr, '&buftype')
        if index(['', 'help', 'quickfix'], kind) < 0
          let reason = 'The editor has a terminal or special buffer'
          break
        endif
      endfor
    endif
    let processes = []
    if exists('*job_info')
      for job in job_info()
        if job_status(job) ==# 'run'
          call add(processes, job_info(job).process)
        endif
      endfor
    endif
    if !empty(processes) | let reason = 'A background editor job is still running' | endif
    call s:reply({'token': request[0], 'phase': request[1], 'pid': getpid(), 'reason': reason, 'processes': processes, 'services': []})
    if request[1] ==# 'commit' && empty(reason) && empty(request[3]) && filereadable(s:dir . '/request')
      let save_aw = &autowrite
      let save_awa = &autowriteall
      let save_confirm = &confirm
      try
        set noautowrite noautowriteall noconfirm
        qall
      catch
        call s:reply({'token': request[0], 'phase': request[1], 'pid': getpid(), 'reason': 'The editor refused to quit', 'processes': [], 'services': []})
      finally
        let &autowrite = save_aw
        let &autowriteall = save_awa
        let &confirm = save_confirm
      endtry
    endif
  catch
    " A failed check leaves the editor open; Flere times out conservatively.
  endtry
endfunction
call writefile([string(getpid())], s:dir . '/ready')
call setfperm(s:dir . '/ready', 'rw-------')
call writefile([string(getpid())], s:dir . '/jump-ready')
call setfperm(s:dir . '/jump-ready', 'rw-------')
call timer_start(60, function('s:tick'), {'repeat': -1})
