defmodule LineReversal.LRCP do
  use GenServer

  @max_message_len 999
  @retransmission_timeout_ms 3_000
  @session_expiry_timeout_ms 60_000

  @debug false

  defmodule State do
    defstruct [
      :parent,
      :socket,
      sessions: %{},
      session_refs: %{},
      unclaimed_session_refs: [],
      accept_pids: []
    ]
  end

  defmodule Session do
    defstruct [
      :ref,
      :origin,
      :token,
      controlling_pid: nil,
      received: 0,
      recv_buf: "",
      sent: 0,
      send_buf: "",
      peer_received: 0
    ]
  end

  ## Client

  def listen(port) do
    parent = self()
    GenServer.start_link(__MODULE__, {parent, port})
  end

  def accept(lrcp) do
    GenServer.cast(lrcp, {:accept, self()})

    receive do
      {:lrcp_session, lrcp, session_ref} ->
        {:ok, lrcp, session_ref}
    end
  end

  def send(lrcp, session, data) do
    GenServer.cast(lrcp, {:send, session, data})
  end

  def session_controlling_process(lrcp, session_ref, pid) do
    GenServer.cast(lrcp, {:session_controlling_process, session_ref, pid})
  end

  ## Server

  @impl true
  def init({parent, port}) do
    {:ok, socket} = :gen_udp.open(port, [:binary, reuseaddr: true])
    IO.puts("Listening on port #{port}")
    state = %State{parent: parent, socket: socket}
    {:ok, state}
  end

  @impl true
  def handle_cast({:accept, pid}, state = %State{unclaimed_session_refs: []}) do
    {:noreply, update_in(state.accept_pids, &List.insert_at(&1, -1, pid))}
  end

  @impl true
  def handle_cast(
        {:accept, pid},
        state = %State{unclaimed_session_refs: [session_ref | rest_session_refs]}
      ) do
    notify_new_session(pid, session_ref)
    {:noreply, %State{state | unclaimed_session_refs: rest_session_refs}}
  end

  @impl true
  def handle_cast({:session_controlling_process, session_ref, pid}, state) do
    case state.session_refs[session_ref] do
      nil ->
        {:noreply, state}

      session ->
        # Send all data that was buffered until the process took control of the session
        {recv_buf, state} = get_and_update_in(state.sessions[session].recv_buf, &{&1, ""})
        notify_data(pid, recv_buf)
        {:noreply, put_in(state.sessions[session].controlling_pid, pid)}
    end
  end

  @impl true
  def handle_cast({:send, session_ref, _data}, state = %State{session_refs: session_refs})
      when not is_map_key(session_refs, session_ref) do
    {:noreply, state}
  end

  @impl true
  def handle_cast(
        {:send, session_ref, data},
        state = %State{socket: socket, session_refs: session_refs, sessions: sessions}
      ) do
    session_num = session_refs[session_ref]
    session = send_data(sessions[session_num], socket, data)
    {:noreply, put_in(state.sessions[session_num], session)}
  end

  @impl true
  def handle_info({:udp, _socket, ip, port, packet}, state) do
    if @debug do
      IO.inspect(packet, label: "Received")
    end

    state =
      packet
      |> parse_packet()
      |> handle_packet(state, {ip, port})

    {:noreply, state}
  end

  @impl true
  def handle_info(
        {:maybe_retransmit, session_ref, _pos, _packet},
        state = %State{session_refs: session_refs}
      )
      when not is_map_key(session_refs, session_ref) do
    {:noreply, state}
  end

  @impl true
  def handle_info(
        {:maybe_retransmit, session_ref, pos, packet},
        state = %State{socket: socket, session_refs: session_refs, sessions: sessions}
      ) do
    %Session{origin: origin, peer_received: peer_received} = sessions[session_refs[session_ref]]

    if peer_received <= pos do
      resend_data_packet(socket, origin, session_ref, pos, packet)
    end

    {:noreply, state}
  end

  @impl true
  def handle_info(
        {:maybe_expire_session, session_ref, _pos},
        state = %State{session_refs: session_refs}
      )
      when not is_map_key(session_refs, session_ref) do
    {:noreply, state}
  end

  @impl true
  def handle_info(
        {:maybe_expire_session, session_ref, pos},
        state = %State{session_refs: session_refs, sessions: sessions}
      ) do
    session_num = session_refs[session_ref]
    %Session{peer_received: peer_received} = sessions[session_num]

    state =
      if peer_received <= pos do
        close_session(state, session_num)
      else
        state
      end

    {:noreply, state}
  end

  ## Private Functions

  defp notify_new_session(pid, session_ref) do
    lrcp = self()
    send(pid, {:lrcp_session, lrcp, session_ref})
  end

  defp notify_data(pid, data) do
    lrcp = self()
    send(pid, {:lrcp, lrcp, data})
  end

  defp notify_session_close(pid, session_ref) do
    lrcp = self()
    send(pid, {:lrcp_close, lrcp, session_ref})
  end

  defp udp_send(socket, recipient, data) do
    if @debug do
      IO.inspect(data, label: "Sending")
    end

    :gen_udp.send(socket, recipient, data)
  end

  defp register_session(state = %State{sessions: sessions}, session, _origin)
       when is_map_key(sessions, session) do
    state
  end

  defp register_session(state = %State{}, session, origin) do
    ref = make_ref()
    state = put_in(state.session_refs[ref], session)
    state = put_in(state.sessions[session], %Session{ref: ref, origin: origin, token: session})

    {accept_pid, state} =
      get_and_update_in(state.accept_pids, fn
        [] -> {nil, []}
        [pid | rest] -> {pid, rest}
      end)

    # Not made for processes that somehow die while accepting new sessions
    if not is_nil(accept_pid) do
      notify_new_session(accept_pid, ref)
      state
    else
      update_in(state.unclaimed_session_refs, &List.insert_at(&1, -1, ref))
    end
  end

  defp close_session(state = %State{sessions: sessions}, session)
       when not is_map_key(sessions, session) do
    state
  end

  defp close_session(state = %State{socket: socket}, session) do
    {%Session{ref: session_ref, controlling_pid: controlling_pid, origin: origin}, state} =
      pop_in(state.sessions[session])

    udp_send(socket, origin, "/close/#{session}/")

    {_, state} = pop_in(state.session_refs[session_ref])

    if not is_nil(controlling_pid) do
      notify_session_close(controlling_pid, session_ref)

      update_in(state.unclaimed_session_refs, fn refs ->
        Enum.reject(refs, &Kernel.==(&1, session_ref))
      end)
    else
      state
    end
  end

  defp chunk_data(data, chunks \\ []) do
    # Worst Case overhead of data message without data
    max_message_overhead = 29

    # Worst Case acceptable unescaped data length (escaped data length could
    # double If all characters are escaped)
    max_unescaped_data_len = floor((@max_message_len - max_message_overhead) / 2)

    if byte_size(data) > max_unescaped_data_len do
      chunk = binary_slice(data, 0, max_unescaped_data_len)
      rest = binary_slice(data, max_unescaped_data_len, byte_size(data) - max_unescaped_data_len)
      chunk_data(rest, [chunk | chunks])
    else
      Enum.reverse([data | chunks])
    end
  end

  defp send_data(session = %Session{}, socket, data) do
    # This is For sending new data
    chunks = chunk_data(data)
    send_chunks(session, socket, chunks)
  end

  defp send_chunks(session = %Session{}, _socket, []) do
    session
  end

  defp send_chunks(
         session = %Session{
           origin: origin,
           ref: ref,
           token: token,
           sent: sent,
           send_buf: send_buf
         },
         socket,
         [
           chunk | rest_chunks
         ]
       ) do
    chunk_len = byte_size(chunk)
    escaped_chunk = escape_message(chunk)
    packet = "/data/#{token}/#{sent}/#{escaped_chunk}/"
    udp_send(socket, origin, packet)

    schedule_retransmission(ref, sent, packet)
    schedule_session_expiry(ref, sent)

    send_chunks(
      %Session{session | sent: sent + chunk_len, send_buf: send_buf <> chunk},
      socket,
      rest_chunks
    )
  end

  defp schedule_retransmission(session_ref, sent, packet) do
    Process.send_after(
      self(),
      {:maybe_retransmit, session_ref, sent, packet},
      @retransmission_timeout_ms
    )
  end

  defp schedule_session_expiry(ref, sent) do
    Process.send_after(self(), {:maybe_expire_session, ref, sent}, @session_expiry_timeout_ms)
  end

  defp resend_data_packet(socket, origin, session_ref, pos, packet) do
    udp_send(socket, origin, packet)
    schedule_retransmission(session_ref, pos, packet)
  end

  def handle_packet(
        {:ok, {:connect, session}},
        state = %State{socket: socket},
        origin
      ) do
    state = register_session(state, session, origin)
    :ok = udp_send(socket, origin, "/ack/#{session}/0/")
    state
  end

  def handle_packet(
        {:ok, {:data, session, _pos, _data}},
        state = %State{socket: socket, sessions: sessions},
        origin
      )
      when not is_map_key(sessions, session) do
    udp_send(socket, origin, "/close/#{session}/")
    state
  end

  def handle_packet(
        {:ok, {:data, session_num, pos, data}},
        state = %State{socket: socket, sessions: sessions},
        _
      ) do
    session =
      case sessions[session_num] do
        session = %Session{received: ^pos, controlling_pid: controlling_pid} ->
          length = byte_size(data)
          session = update_in(session.received, &Kernel.+(&1, length))

          session =
            if is_nil(controlling_pid) do
              update_in(session.recv_buf, &Kernel.<>(&1, data))
            else
              notify_data(controlling_pid, data)
              session
            end

          session

        session = %Session{} ->
          # Not received all data up until pos, just retransmit previous ACK
          session
      end

    udp_send(socket, session.origin, "/ack/#{session_num}/#{session.received}/")

    put_in(state.sessions[session_num], session)
  end

  def handle_packet(
        {:ok, {:ack, session_num, len}},
        state = %State{socket: socket, sessions: sessions},
        origin
      ) do
    case sessions[session_num] do
      nil ->
        udp_send(socket, origin, "/close/#{session_num}/")
        state

      %Session{peer_received: peer_received} when len <= peer_received ->
        # Assume duplicate ACK
        state

      %Session{sent: sent} when len > sent ->
        # Peer misbehaving
        close_session(state, session_num)

      %Session{peer_received: peer_received} ->
        # Some or all not previously ACKed data is ACKed

        new_acked = len - peer_received
        session = state.sessions[session_num]
        session = put_in(session.peer_received, len)

        session =
          update_in(
            session.send_buf,
            &binary_slice(&1, new_acked, byte_size(&1) - new_acked)
          )

        to_retransmit = session.send_buf

        session =
          if to_retransmit != "" do
            session = %Session{
              session
              | send_buf: "",
                sent: session.sent - byte_size(to_retransmit)
            }

            send_data(session, socket, to_retransmit)
          else
            session
          end

        put_in(state.sessions[session_num], session)
    end
  end

  def handle_packet(
        {:ok, {:close, session}},
        state = %State{socket: socket, sessions: sessions},
        origin
      )
      when not is_map_key(sessions, session) do
    udp_send(socket, origin, "/close/#{session}/")
    state
  end

  def handle_packet({:ok, {:close, session}}, state = %State{}, _origin) do
    close_session(state, session)
  end

  # Silently ignore errors
  def handle_packet({:error, _reason}, state, _) do
    state
  end

  def parse_packet(packet) do
    cond do
      byte_size(packet) > 1000 ->
        {:error, :too_long}

      not String.starts_with?(packet, "/") or not String.ends_with?(packet, "/") or
          byte_size(packet) < 2 ->
        {:error, :not_slash_fenced}

      true ->
        # Strip slashes at start and end
        packet = binary_slice(packet, 1, byte_size(packet) - 2)

        case String.split(packet, "/") do
          ["connect", session_str] ->
            with {:ok, session} <- parse_num(session_str) do
              {:ok, {:connect, session}}
            end

          ["data", session_str, pos_str | data_list] when data_list != [] ->
            data = Enum.join(data_list, "/")

            with {:ok, session} <- parse_num(session_str),
                 {:ok, pos} <- parse_num(pos_str),
                 :ok <- validate_data(data) do
              {:ok, {:data, session, pos, unescape_message(data)}}
            end

          ["ack", session_str, len_str] ->
            with {:ok, session} <- parse_num(session_str), {:ok, len} <- parse_num(len_str) do
              {:ok, {:ack, session, len}}
            end

          ["close", session_str] ->
            with {:ok, session} <- parse_num(session_str) do
              {:ok, {:close, session}}
            end

          _ ->
            {:error, :wrong_format}
        end
    end
  end

  defp unescape_message(message) do
    message
    |> String.replace("\\\\", "\\")
    |> String.replace("\\/", "/")
  end

  defp escape_message(message) do
    message
    |> String.replace("\\", "\\\\")
    |> String.replace("/", "\\/")
  end

  def validate_data(data) do
    if escape_message(unescape_message(data)) == data do
      :ok
    else
      {:error, :invalid_escaping}
    end
  end

  defp parse_num(num_str) do
    case Integer.parse(num_str) do
      {num, ""} when num >= 0 and num < 2_147_483_648 ->
        {:ok, num}

      _ ->
        {:error, :number_parse_error}
    end
  end
end
