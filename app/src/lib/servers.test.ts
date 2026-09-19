import { describe, expect, test } from 'vitest'
import {
  clashes,
  forgotPasswordUrl,
  hostProblem,
  labelOf,
  newServer,
  parsePorts,
  serverId,
  serverName,
} from './servers'

describe('servers', () => {
  test('are known by their host, trimmed and lowercased', () => {
    expect(serverId(' Server4.BeyondAllReason.info ')).toBe(
      'server4.beyondallreason.info',
    )
  })

  test('are called by their name, else their host', () => {
    expect(serverName({ ...newServer('rapid.example'), name: 'Rapid' })).toBe(
      'Rapid',
    )
    expect(serverName(newServer('rapid.example'))).toBe('rapid.example')
  })

  test('reset a forgotten password on their website, else on their host', () => {
    expect(forgotPasswordUrl(newServer('server4.beyondallreason.info'))).toBe(
      'https://server4.beyondallreason.info/forgot_password',
    )
    expect(
      forgotPasswordUrl({
        ...newServer('server.example.com'),
        website: 'https://server.example.com/',
      }),
    ).toBe('https://server.example.com/forgot_password')
  })
})

describe('adding a server', () => {
  test('takes a bare host that is not listed yet', () => {
    expect(hostProblem('server.example.com', ['server4'])).toBeNull()
  })

  test('refuses nothing, a port or a path, and a server already listed', () => {
    expect(hostProblem('  ', [])).toBe('a host is needed')
    expect(hostProblem('example.org:8200', [])).toContain('ports')
    expect(hostProblem('https://example.org', [])).toContain('ports')
    expect(hostProblem('Server4', ['server4'])).toBe(
      'that server is already listed',
    )
  })
})

describe('ports as typed', () => {
  test('are read with commas or spaces, once each', () => {
    expect(parsePorts('8200, 8201')).toEqual([8200, 8201])
    expect(parsePorts('8200 8201 8200')).toEqual([8200, 8201])
  })

  test('are nothing until every one of them is a port', () => {
    expect(parsePorts('')).toBeNull()
    expect(parsePorts('8200, x')).toBeNull()
    expect(parsePorts('70000')).toBeNull()
    expect(parsePorts('0')).toBeNull()
  })

  test('are labelled by their entry, else by their id', () => {
    const listed = [{ ...newServer('Rapid.example'), name: 'Rapid' }]
    expect(labelOf(listed, 'rapid.example')).toBe('Rapid')
    expect(labelOf(listed, 'gone.example')).toBe('gone.example')
  })

  test('tag a name only where two servers share it', () => {
    const clashing = clashes([
      { server: 'bar', name: 'main' },
      { server: 'rapid', name: 'main' },
      { server: 'bar', name: 'bob' },
      { server: 'bar', name: 'bob' },
      { server: null, name: 'Battle room' },
    ])
    expect([...clashing]).toEqual(['main'])
  })

  test('tag nothing while there is one server', () => {
    const one = clashes([
      { server: 'bar', name: 'main' },
      { server: 'bar', name: 'Server' },
      { server: 'bar', name: 'main' },
    ])
    expect(one.size).toBe(0)
  })
})
