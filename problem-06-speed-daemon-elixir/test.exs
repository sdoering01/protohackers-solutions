{:ok, c1} = :gen_tcp.connect({127, 0, 0, 1}, 10000, [:binary])
:gen_tcp.send(c1, <<0x80, 0x00, 0x7B, 0x00, 0x08, 0x00, 0x3C>>)
:gen_tcp.send(c1, <<0x20, 0x04, 0x55, 0x4E, 0x31, 0x58, 0x00, 0x00, 0x00, 0x00>>)

{:ok, c2} = :gen_tcp.connect({127, 0, 0, 1}, 10000, [:binary])
:gen_tcp.send(c2, <<0x80, 0x00, 0x7B, 0x00, 0x09, 0x00, 0x3C>>)
:gen_tcp.send(c2, <<0x20, 0x04, 0x55, 0x4E, 0x31, 0x58, 0x00, 0x00, 0x00, 0x2D>>)

{:ok, c3} = :gen_tcp.connect({127, 0, 0, 1}, 10000, [:binary, active: false])
:gen_tcp.send(c3, <<0x81, 0x01, 0x00, 0x7B>>)

IO.puts("Waiting for packet")

{:ok, packet} = :gen_tcp.recv(c3, 22)

IO.inspect(packet, label: "Packet")

IO.inspect(
  packet ==
    <<0x21, 0x04, 0x55, 0x4E, 0x31, 0x58, 0x00, 0x7B, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00,
      0x09, 0x00, 0x00, 0x00, 0x2D, 0x1F, 0x40>>,
  label: "Is correct packet"
)
